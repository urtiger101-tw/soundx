/// Convert seconds to frame count at the given sample rate.
///
/// Negative and NaN inputs give 0 and huge inputs saturate at `usize::MAX`
/// (float-to-int casts saturate), so callers that add or multiply the result
/// must use checked or saturating arithmetic.
pub fn seconds_to_frames(seconds: f32, sample_rate: u32) -> usize {
    (seconds as f64 * sample_rate as f64).round().max(0.0) as usize
}
