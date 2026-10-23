//! Som de confirmação ao salvar um clipe: um WAV curto gerado aqui e tocado pelo
//! sistema (PlaySound no Windows; pw-play/paplay/aplay no Linux).

use std::f32::consts::PI;
use std::path::Path;

const RATE: u32 = 44_100;
/// Muda o nome se o som mudar, para não reaproveitar o arquivo antigo do cache.
const FILE_NAME: &str = "clipe-salvo-v1.wav";

/// Toca em segundo plano; falhas são ignoradas (é só um aviso sonoro).
pub fn play_saved(cache_dir: &Path) {
    let path = cache_dir.join(FILE_NAME);
    std::thread::spawn(move || {
        if !path.is_file() && std::fs::write(&path, chime_wav()).is_err() {
            return;
        }
        play(&path);
    });
}

/// Duas notas curtas subindo (si → mi), com ataque rápido e decaimento.
fn chime_wav() -> Vec<u8> {
    let notes = [(987.8_f32, 0.0_f32), (1318.5, 0.085)];
    let len = (RATE as f32 * 0.42) as usize;
    let mut samples = vec![0f32; len];
    for (freq, start) in notes {
        let first = (start * RATE as f32) as usize;
        for (i, sample) in samples.iter_mut().enumerate().skip(first) {
            let t = (i - first) as f32 / RATE as f32;
            let envelope = (t * 400.0).min(1.0) * (-t * 11.0).exp();
            *sample += (2.0 * PI * freq * t).sin() * envelope * 0.28;
        }
    }
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect();

    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // tamanho do bloco fmt
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes por segundo
    wav.extend_from_slice(&2u16.to_le_bytes()); // bytes por amostra
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

#[cfg(windows)]
fn play(path: &Path) {
    use std::ffi::c_void;

    const SND_NODEFAULT: u32 = 0x0002;
    const SND_FILENAME: u32 = 0x0002_0000;

    #[link(name = "winmm")]
    extern "system" {
        fn PlaySoundW(sound: *const u16, module: *mut c_void, flags: u32) -> i32;
    }

    let name = crate::util::wide(path);
    // Síncrono: já estamos numa thread própria.
    unsafe { PlaySoundW(name.as_ptr(), std::ptr::null_mut(), SND_FILENAME | SND_NODEFAULT) };
}

#[cfg(not(windows))]
fn play(path: &Path) {
    use std::process::Stdio;
    let players: [(&str, &[&str]); 3] = [("pw-play", &[]), ("paplay", &[]), ("aplay", &["-q"])];
    for (program, args) in players {
        let ok = crate::util::command(program)
            .args(args)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_header() {
        let wav = super::chime_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[36..40], b"data");
        let riff_len = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(riff_len + 8, wav.len());
        assert_eq!(data_len + 44, wav.len());
    }
}
