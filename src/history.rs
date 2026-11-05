//! Time series for every sensor: one sample per second for the last hour, then
//! one-minute buckets for the last day. Series are aligned on their newest sample.

use std::collections::{HashMap, VecDeque};

pub type SeriesId = usize;

const FINE_SAMPLES: usize = 3600;
const COARSE_SAMPLES: usize = 1440;
const COARSE_SECONDS: u32 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub now: Option<f32>,
    pub high: Option<f32>,
    pub avg: Option<f32>,
}

#[derive(Clone, Copy)]
struct Bucket {
    high: f32,
    avg: f32,
}

/// Running high and mean over the samples seen so far, ignoring gaps.
#[derive(Clone, Copy, Default)]
struct Accumulator {
    high: Option<f32>,
    sum: f64,
    count: u32,
}

impl Accumulator {
    fn add(&mut self, high: f32, avg: f32) {
        if avg.is_nan() {
            return;
        }
        self.high = Some(self.high.map_or(high, |h| h.max(high)));
        self.sum += avg as f64;
        self.count += 1;
    }

    fn avg(&self) -> Option<f32> {
        (self.count > 0).then(|| (self.sum / self.count as f64) as f32)
    }
}

#[derive(Default)]
pub struct Series {
    fine: VecDeque<f32>,
    coarse: VecDeque<Bucket>,
    open_bucket: Accumulator,
    open_samples: u32,
}

impl Series {
    fn push(&mut self, value: f32) {
        if self.fine.len() == FINE_SAMPLES {
            self.fine.pop_front();
        }
        self.fine.push_back(value);

        self.open_bucket.add(value, value);
        self.open_samples += 1;
        if self.open_samples == COARSE_SECONDS {
            if self.coarse.len() == COARSE_SAMPLES {
                self.coarse.pop_front();
            }
            self.coarse.push_back(Bucket {
                high: self.open_bucket.high.unwrap_or(f32::NAN),
                avg: self.open_bucket.avg().unwrap_or(f32::NAN),
            });
            self.open_bucket = Accumulator::default();
            self.open_samples = 0;
        }
    }

    pub fn latest(&self) -> Option<f32> {
        self.fine.back().copied().filter(|v| !v.is_nan())
    }

    /// True once the sensor has produced at least one real reading.
    pub fn has_data(&self) -> bool {
        self.fine.iter().any(|v| !v.is_nan()) || self.coarse.iter().any(|b| !b.avg.is_nan())
    }

    pub fn stats(&self, seconds: u32) -> Stats {
        let mut total = Accumulator::default();
        if seconds as usize <= FINE_SAMPLES {
            for &v in self.fine.iter().rev().take(seconds as usize) {
                total.add(v, v);
            }
        } else {
            let buckets = (seconds / COARSE_SECONDS) as usize;
            for b in self.coarse.iter().rev().take(buckets) {
                total.add(b.high, b.avg);
            }
            if let Some(high) = self.open_bucket.high {
                total.add(high, self.open_bucket.avg().unwrap_or(f32::NAN));
            }
        }
        Stats { now: self.latest(), high: total.high, avg: total.avg() }
    }

    /// Evenly spaced values across the window, oldest first, `NaN` where there is
    /// no data. `peak` keeps the highest sample of each span instead of the mean.
    pub fn points(&self, seconds: u32, max_points: usize, peak: bool) -> Vec<f32> {
        if seconds as usize <= FINE_SAMPLES {
            let samples: Vec<f32> = self.fine.iter().copied().collect();
            resample(&samples, seconds as usize, max_points, peak)
        } else {
            let pick = |b: &Bucket| if peak { b.high } else { b.avg };
            let samples: Vec<f32> = self.coarse.iter().map(pick).collect();
            resample(&samples, (seconds / COARSE_SECONDS) as usize, max_points, peak)
        }
    }
}

fn resample(samples: &[f32], window: usize, max_points: usize, peak: bool) -> Vec<f32> {
    let count = window.min(max_points).max(2);
    (0..count)
        .map(|i| {
            // Span of this point, measured in samples from the start of the window.
            let start = i * window / count;
            let end = ((i + 1) * window / count).max(start + 1);
            let mut span = Accumulator::default();
            for offset in start..end.min(window) {
                let age = window - 1 - offset;
                if let Some(index) = samples.len().checked_sub(age + 1) {
                    span.add(samples[index], samples[index]);
                }
            }
            if peak { span.high.unwrap_or(f32::NAN) } else { span.avg().unwrap_or(f32::NAN) }
        })
        .collect()
}

#[derive(Default)]
pub struct History {
    series: Vec<Series>,
    pending: Vec<f32>,
    index: HashMap<String, SeriesId>,
}

impl History {
    /// Looks up a series by its stable key, creating it if needed, so that data
    /// survives the helper reconnecting or re-announcing its sensors.
    pub fn id(&mut self, key: &str) -> SeriesId {
        if let Some(&id) = self.index.get(key) {
            return id;
        }
        let id = self.series.len();
        self.series.push(Series::default());
        self.pending.push(f32::NAN);
        self.index.insert(key.to_owned(), id);
        id
    }

    pub fn set(&mut self, id: SeriesId, value: f32) {
        self.pending[id] = value;
    }

    /// The value set for the sample being assembled.
    pub fn pending(&self, id: SeriesId) -> f32 {
        self.pending[id]
    }

    /// Closes the current sample. Series that were not set record a gap.
    pub fn commit(&mut self) {
        for (series, value) in self.series.iter_mut().zip(&mut self.pending) {
            series.push(*value);
            *value = f32::NAN;
        }
    }

    pub fn series(&self, id: SeriesId) -> &Series {
        &self.series[id]
    }

    pub fn clear(&mut self) {
        for series in &mut self.series {
            *series = Series::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(values: &[f32]) -> Series {
        let mut series = Series::default();
        values.iter().for_each(|&v| series.push(v));
        series
    }

    #[test]
    fn stats_cover_only_the_window() {
        let stats = filled(&[90.0, 10.0, 20.0, 30.0]).stats(3);
        assert_eq!(stats.now, Some(30.0));
        assert_eq!(stats.high, Some(30.0));
        assert_eq!(stats.avg, Some(20.0));
    }

    #[test]
    fn gaps_are_ignored_by_stats() {
        let stats = filled(&[10.0, f32::NAN, 30.0]).stats(60);
        assert_eq!(stats.high, Some(30.0));
        assert_eq!(stats.avg, Some(20.0));
    }

    #[test]
    fn missing_latest_reading_is_not_reported_as_current() {
        assert_eq!(filled(&[10.0, f32::NAN]).stats(60).now, None);
        assert_eq!(filled(&[f32::NAN]).stats(60).high, None);
    }

    #[test]
    fn long_windows_use_minute_buckets_and_keep_the_peak() {
        let mut series = Series::default();
        for i in 0..7200 {
            series.push(if i == 100 { 99.0 } else { 40.0 });
        }
        // The spike is older than the fine ring, so only a bucket can remember it.
        let stats = series.stats(6 * 3600);
        assert_eq!(stats.high, Some(99.0));
        assert!((stats.avg.unwrap() - 40.0).abs() < 0.1);
        assert_eq!(series.stats(3600).high, Some(40.0));
    }

    #[test]
    fn points_are_right_aligned_with_gaps_before_the_first_sample() {
        let points = filled(&[1.0, 2.0, 3.0]).points(5, 100, false);
        assert_eq!(points.len(), 5);
        assert!(points[0].is_nan() && points[1].is_nan());
        assert_eq!(&points[2..], &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn downsampling_can_keep_peaks_or_means() {
        let series = filled(&[0.0, 10.0, 0.0, 30.0]);
        assert_eq!(series.points(4, 2, true), vec![10.0, 30.0]);
        assert_eq!(series.points(4, 2, false), vec![5.0, 15.0]);
    }

    #[test]
    fn unset_series_record_a_gap_and_stay_aligned() {
        let mut history = History::default();
        let (a, b) = (history.id("a"), history.id("b"));
        history.set(a, 1.0);
        history.set(b, 1.0);
        history.commit();
        history.set(a, 2.0);
        history.commit();
        assert_eq!(history.series(a).latest(), Some(2.0));
        assert_eq!(history.series(b).latest(), None);
        assert!(history.series(b).has_data());
    }
}
