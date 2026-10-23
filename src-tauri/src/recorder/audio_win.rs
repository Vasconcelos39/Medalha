//! Áudio no Windows: o ffmpeg não tem loopback WASAPI, então capturamos aqui
//! (som do sistema + microfone), mixamos em 48 kHz estéreo e mandamos PCM f32
//! pelo stdin do ffmpeg. A saída é cadenciada pelo relógio: quando não toca
//! nada (o loopback não entrega amostras), sai silêncio e o vídeo continua em sincronia.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use std::collections::VecDeque;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const RATE: u32 = 48_000;
/// Folga acumulada antes de consumir uma fonte, para não picotar.
const PRIME_FRAMES: usize = RATE as usize * 40 / 1000;
/// Limite de atraso de uma fonte (descarta o excesso).
const MAX_FRAMES: usize = RATE as usize / 4;

type Frame = [f32; 2];

#[derive(Default)]
struct Source {
    buf: Mutex<VecDeque<Frame>>,
}

pub struct AudioPump {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AudioPump {
    pub fn start(mut stdin: ChildStdin, system: bool, mic: bool) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let host = cpal::default_host();
            let mut sources = Vec::new();
            let mut streams = Vec::new();
            let mut open_device = |dev: Option<cpal::Device>, loopback: bool| {
                let Some(dev) = dev else { return };
                match open(&dev, loopback) {
                    Ok((stream, src)) => {
                        if stream.play().is_ok() {
                            streams.push(stream);
                            sources.push(src);
                        }
                    }
                    Err(e) => eprintln!("medalha: áudio indisponível: {e}"),
                }
            };
            if system {
                open_device(host.default_output_device(), true);
            }
            if mic {
                open_device(host.default_input_device(), false);
            }
            pump(&mut stdin, &sources, &stop_flag);
            drop(streams);
        });
        Self { stop, thread: Some(thread) }
    }

    /// Chame depois de matar o ffmpeg (o pipe quebrado destrava a escrita).
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn open(dev: &cpal::Device, loopback: bool) -> Result<(cpal::Stream, Arc<Source>), String> {
    let supported = if loopback {
        dev.default_output_config()
    } else {
        dev.default_input_config()
    }
    .map_err(|e| e.to_string())?;
    let channels = supported.channels() as usize;
    let rate = supported.sample_rate().0;
    let config: cpal::StreamConfig = supported.config();
    let src = Arc::new(Source::default());
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(dev, &config, channels, rate, src.clone()),
        cpal::SampleFormat::I16 => build::<i16>(dev, &config, channels, rate, src.clone()),
        cpal::SampleFormat::U16 => build::<u16>(dev, &config, channels, rate, src.clone()),
        cpal::SampleFormat::I32 => build::<i32>(dev, &config, channels, rate, src.clone()),
        other => Err(format!("formato de áudio {other:?} não suportado")),
    }?;
    Ok((stream, src))
}

fn build<T>(
    dev: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    rate: u32,
    src: Arc<Source>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let mut resampler = Resampler::new(rate, RATE);
    let mut frames: Vec<Frame> = Vec::new();
    let mut out: Vec<Frame> = Vec::new();
    dev.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            frames.clear();
            for chunk in data.chunks_exact(channels) {
                let l = f32::from_sample(chunk[0]);
                let r = if channels > 1 { f32::from_sample(chunk[1]) } else { l };
                frames.push([l, r]);
            }
            out.clear();
            resampler.process(&frames, &mut out);
            let mut buf = src.buf.lock().unwrap();
            buf.extend(out.iter().copied());
            let excess = buf.len().saturating_sub(MAX_FRAMES);
            buf.drain(..excess);
        },
        |e| eprintln!("medalha: erro no stream de áudio: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}

fn pump(out: &mut ChildStdin, sources: &[Arc<Source>], stop: &AtomicBool) {
    let start = Instant::now();
    let mut written: u64 = 0;
    let mut primed = vec![false; sources.len()];
    let mut mix: Vec<Frame> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(10));
        let target = (start.elapsed().as_secs_f64() * RATE as f64) as u64;
        if target <= written {
            continue;
        }
        let n = (target - written) as usize;
        mix.clear();
        mix.resize(n, [0.0; 2]);
        for (src, primed) in sources.iter().zip(primed.iter_mut()) {
            let mut buf = src.buf.lock().unwrap();
            if !*primed {
                if buf.len() < PRIME_FRAMES {
                    continue;
                }
                *primed = true;
            }
            for frame in mix.iter_mut() {
                match buf.pop_front() {
                    Some(s) => {
                        frame[0] += s[0];
                        frame[1] += s[1];
                    }
                    None => {
                        *primed = false;
                        break;
                    }
                }
            }
        }
        bytes.clear();
        for frame in &mix {
            for s in frame {
                bytes.extend_from_slice(&s.clamp(-1.0, 1.0).to_le_bytes());
            }
        }
        if out.write_all(&bytes).is_err() {
            break;
        }
        written += n as u64;
    }
}

/// Reamostragem linear (suficiente para 44.1 kHz → 48 kHz).
struct Resampler {
    step: f64,
    pos: f64,
    last: Option<Frame>,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self { step: from as f64 / to as f64, pos: 0.0, last: None }
    }

    fn process(&mut self, input: &[Frame], out: &mut Vec<Frame>) {
        if self.step == 1.0 {
            out.extend_from_slice(input);
            return;
        }
        let mut input = input;
        let last = match self.last {
            Some(l) => l,
            None => {
                let Some((&first, rest)) = input.split_first() else { return };
                input = rest;
                first
            }
        };
        // Índice virtual 0 = `last`, k = input[k - 1].
        let n = input.len();
        while self.pos < n as f64 {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            let a = if i == 0 { last } else { input[i - 1] };
            let b = input[i];
            out.push([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
            self.pos += self.step;
        }
        self.last = Some(input.last().copied().unwrap_or(last));
        self.pos -= n as f64;
    }
}
