//! Night audio: a compressor between the apps and the output, so explosions
//! come down and voices come up and nobody reaches for the volume all film.
//!
//! tvd runs PipeWire's filter-chain as its own `pipewire -c` process: a
//! high-pass at 80 Hz (rumble a living room at night does not need), the SWH
//! `sc4` compressor, and the SWH lookahead limiter so the makeup gain never
//! clips. Its sink becomes the default while it runs and plays into the output
//! that was the default before. The plugins come from the `swh-plugins`
//! package; without them night audio refuses to switch on and says so.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::audio;

/// The compressor's sink, by node name.
pub const SINK: &str = "rustv_night_audio";
/// The LADSPA plugins the graph needs, from `swh-plugins`.
const PLUGINS: [&str; 2] = ["sc4_1882", "fast_lookahead_limiter_1913"];

/// Whether the plugins are installed where PipeWire looks for LADSPA plugins.
pub fn plugins_present() -> bool {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("LADSPA_PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    dirs.extend(["/usr/lib/ladspa", "/usr/lib64/ladspa", "/usr/local/lib/ladspa"].map(PathBuf::from));
    dirs.extend(std::fs::read_dir("/usr/lib").into_iter().flatten().flatten().map(|entry| entry.path().join("ladspa")));
    PLUGINS.iter().all(|plugin| dirs.iter().any(|dir| dir.join(format!("{plugin}.so")).is_file()))
}

/// The error for a remote when the plugins are missing.
pub fn missing() -> anyhow::Error {
    crate::i18n::error("error.night_audio.plugins", &[])
}

/// The filter-chain config: the graph, its sink, and its output into `target` (a node name).
pub fn conf(target: &str) -> String {
    format!(
        r#"# Written by tvd for night audio; see night_audio.rs.
context.properties = {{ log.level = 1 }}
context.spa-libs = {{
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}}
context.modules = [
    {{ name = libpipewire-module-rt args = {{ }} flags = [ ifexists nofail ] }}
    {{ name = libpipewire-module-protocol-native }}
    {{ name = libpipewire-module-client-node }}
    {{ name = libpipewire-module-adapter }}
    {{ name = libpipewire-module-filter-chain
        args = {{
            node.description = "Night audio"
            media.name = "Night audio"
            filter.graph = {{
                nodes = [
                    {{ type = builtin name = rumble_l label = bq_highpass control = {{ "Freq" = 80.0 }} }}
                    {{ type = builtin name = rumble_r label = bq_highpass control = {{ "Freq" = 80.0 }} }}
                    {{ type = ladspa name = compressor plugin = sc4_1882 label = sc4
                      control = {{ "RMS/peak" = 0.5 "Attack time (ms)" = 15.0 "Release time (ms)" = 300.0
                                  "Threshold level (dB)" = -30.0 "Ratio (1:n)" = 6.0 "Knee radius (dB)" = 6.0
                                  "Makeup gain (dB)" = 12.0 }} }}
                    {{ type = ladspa name = limiter plugin = fast_lookahead_limiter_1913 label = fastLookaheadLimiter
                      control = {{ "Input gain (dB)" = 0.0 "Limit (dB)" = -1.0 "Release time (s)" = 0.1 }} }}
                ]
                links = [
                    {{ output = "rumble_l:Out" input = "compressor:Left input" }}
                    {{ output = "rumble_r:Out" input = "compressor:Right input" }}
                    {{ output = "compressor:Left output" input = "limiter:Input 1" }}
                    {{ output = "compressor:Right output" input = "limiter:Input 2" }}
                ]
                inputs = [ "rumble_l:In" "rumble_r:In" ]
                outputs = [ "limiter:Output 1" "limiter:Output 2" ]
            }}
            capture.props = {{
                node.name = "{SINK}"
                media.class = Audio/Sink
                audio.channels = 2
                audio.position = [ FL FR ]
            }}
            playback.props = {{
                node.name = "{SINK}.output"
                node.passive = true
                audio.channels = 2
                audio.position = [ FL FR ]
                target.object = "{target}"
                node.dont-reconnect = true
            }}
        }}
    }}
]
"#,
        target = target.replace(['"', '\\'], "")
    )
}

struct Running {
    child: Child,
    /// The output it plays into: id and node name.
    behind: (u32, String),
}

/// Night audio, while it runs.
#[derive(Default)]
pub struct NightAudio {
    running: tokio::sync::Mutex<Option<Running>>,
    /// Starting failed for this reason; not tried again until asked anew.
    failed: std::sync::Mutex<Option<String>>,
}

impl NightAudio {
    /// Runs it or stops it. `retry` tries again after a failure (a settings change).
    pub async fn want(&self, on: bool, retry: bool) {
        if retry {
            *self.failed.lock().unwrap() = None;
        }
        let mut running = self.running.lock().await;
        if !on {
            if let Some(old) = running.take() {
                stop(old, true).await;
            }
            return;
        }
        // Still running, and still what plays: nothing to do.
        if let Some(run) = running.as_mut() {
            let alive = run.child.try_wait().ok().flatten().is_none();
            let default = audio::sinks().await.ok().and_then(|(_, default)| default);
            if alive && default.as_deref() == Some(SINK) {
                return;
            }
            // It ended, or another output became the default (headphones connected):
            // start over in front of whatever plays now.
            let old = running.take().expect("checked above");
            stop(old, !alive).await;
        }
        if self.failed.lock().unwrap().is_some() {
            return;
        }
        match start().await {
            Ok(run) => *running = Some(run),
            Err(err) => {
                warn!("night audio: {err:#}");
                *self.failed.lock().unwrap() = Some(format!("{err:#}"));
            }
        }
    }

    /// Plays into the output `id` from now on, for the remote's output picker.
    /// Returns false when night audio is off, and the caller switches as usual.
    pub async fn switch_output(&self, id: u32) -> Result<bool> {
        let mut running = self.running.lock().await;
        let Some(old) = running.take() else { return Ok(false) };
        stop(old, false).await;
        audio::set_output(id).await?;
        *running = Some(start().await?);
        Ok(true)
    }
}

/// Starts it in front of the default output.
async fn start() -> Result<Running> {
    if !plugins_present() {
        return Err(missing());
    }
    let (sinks, default) = audio::sinks().await?;
    let behind = sinks
        .iter()
        .find(|(_, name)| Some(name) == default.as_ref() && name != SINK)
        .map(|(output, name)| (output.id, name.clone()))
        .context("no default audio output to play into")?;

    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = runtime.join("rustv-night-audio.conf");
    std::fs::write(&path, conf(&behind.1)).with_context(|| format!("writing {}", path.display()))?;
    let mut command = Command::new("pipewire");
    command.arg("-c").arg(&path).stdout(std::process::Stdio::null()).kill_on_drop(true);
    // SAFETY: prctl is async-signal-safe; it only asks for SIGTERM when tvd dies,
    // so a crashed tvd never leaves the filter behind.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = command.spawn().context("starting pipewire for night audio")?;

    // Its sink shows up within a moment, or the filter could not start.
    let mut sink = None;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(Some(status)) = child.try_wait() {
            anyhow::bail!("the filter ended at once ({status})");
        }
        let (sinks, _) = audio::sinks().await?;
        if let Some((output, _)) = sinks.iter().find(|(_, name)| name == SINK) {
            sink = Some(output.id);
            break;
        }
    }
    let Some(sink) = sink else {
        let _ = child.kill().await;
        anyhow::bail!("the filter's sink did not appear (see `journalctl --user -u tvd`)");
    };
    // The compressor works on the full signal; the volume stays on the real output.
    audio::set_volume(sink, 1.0).await?;
    audio::set_output(sink).await?;
    audio::set_behind(Some(behind.clone()));
    info!(output = %behind.1, "night audio on");
    Ok(Running { child, behind })
}

/// Stops it; `restore` makes the output behind it the default again.
async fn stop(mut run: Running, restore: bool) {
    audio::set_behind(None);
    if restore {
        if let Err(err) = audio::set_output(run.behind.0).await {
            debug!("night audio: back to {}: {err:#}", run.behind.1);
        }
    }
    let _ = run.child.kill().await;
    info!("night audio off");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graph_plays_into_the_output_behind_it() {
        let text = conf("alsa_output.hdmi");
        assert!(text.contains(r#"target.object = "alsa_output.hdmi""#), "{text}");
        assert!(text.contains(r#"node.name = "rustv_night_audio""#));
        assert!(text.contains(r#"node.name = "rustv_night_audio.output""#));
        for plugin in PLUGINS {
            assert!(text.contains(&format!("plugin = {plugin} ")), "{plugin}");
        }
        // A node name cannot break out of its quotes.
        assert!(conf(r#"a" evil = "b"#).contains(r#"target.object = "a evil = b""#));
    }
}
