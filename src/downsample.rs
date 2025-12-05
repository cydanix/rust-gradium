use base64::Engine;

// FIR filter coefficients for downsampling
const FIR: [f64; 11] = [
    -0.010, -0.020, -0.010,
    0.040, 0.120, 0.180,
    0.120, 0.040, -0.010,
    -0.020, -0.010,
];

// Downsample 48kHz to 24kHz with low-pass filter
pub fn downsample_48_to_24(input: &[u8]) -> Vec<u8> {
    // Convert bytes to i16 samples
    let samples48: Vec<i16> = input
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    let filter_half = FIR.len() / 2;
    let mut out = Vec::with_capacity(samples48.len() / 2);

    // Apply LPF + decimate (take every 2 samples)
    let mut i = filter_half;
    while i < samples48.len().saturating_sub(filter_half) {
        let mut acc = 0.0f64;
        for k in 0..FIR.len() {
            acc += samples48[i - filter_half + k] as f64 * FIR[k];
        }
        acc = acc.clamp(-32768.0, 32767.0);
        out.push(acc as i16);
        i += 2;
    }

    // Pack back into bytes
    out.iter()
        .flat_map(|&s| s.to_le_bytes())
        .collect()
}

pub fn downsample_48_to_24_base64(input: &str) -> String {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(input) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    base64::engine::general_purpose::STANDARD.encode(downsample_48_to_24(&bytes))
}
