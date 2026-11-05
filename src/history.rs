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
    pub low: Option<f32>,
    pub high: Option<f32>,
    pub avg: Option<f32>,
}

#[derive(Clone, Copy)]
struct Bucket {
    low: f32,
    high: f32,
    avg: f32,
}

/// Running low, high and mean over the samples seen so far, ignoring gaps.
#[derive(Clone, Copy, Default)]
struct Accumulator {
    low: Option<f32>,
    high: Option<f32>,
    sum: f64,
    count: u32,
}

impl Accumulator {
    fn add(&mut self, low: f32, high: f32, avg: f32) {
        if avg.is_nan() {
            return;
        }
        self.low = Some(self.low.map_or(low, |l| l.min(low)));
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
    /// Samples and buckets ever received, including ones since dropped.
    fine_total: u64,
    coarse_total: u64,
}

impl Series {
    fn push(&mut self, value: f32) {
        if self.fine.len() == FINE_SAMPLES {
            self.fine.pop_front();
        }
        self.fine.push_back(value);
        self.fine_total += 1;

        self.open_bucket.add(value, value, value);
        self.open_samples += 1;
        if self.open_samples == COARSE_SECONDS {
            if self.coarse.len() == COARSE_SAMPLES {
                self.coarse.pop_front();
            }
            self.coarse_total += 1;
            self.coarse.push_back(Bucket {
                low: self.open_bucket.low.unwrap_or(f32::NAN),
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
                total.add(v, v, v);
            }
        } else {
            let buckets = (seconds / COARSE_SECONDS) as usize;
            for b in self.coarse.iter().rev().take(buckets) {
                total.add(b.low, b.high, b.avg);
            }
            let open = &self.open_bucket;
            if let (Some(low), Some(high)) = (open.low, open.high) {
                total.add(low, high, open.avg().unwrap_or(f32::NAN));
            }
        }
        Stats { now: self.latest(), low: total.low, high: total.high, avg: total.avg() }
    }

    /// Which group of samples, counted since the series began, the first of
    /// `points` stands for. It goes up by one each time the graph slides along a point.
    pub fn first_point(&self, seconds: u32, max_points: usize) -> i64 {
        let (total, window) = if seconds as usize <= FINE_SAMPLES {
            (self.fine_total, seconds as usize)
        } else {
            (self.coarse_total, (seconds / COARSE_SECONDS) as usize)
        };
        let grouping = Grouping::new(total, window, max_points);
        grouping.newest as i64 - (grouping.count as i64 - 1)
    }

    /// Evenly spaced values across the window, oldest first, `NaN` where there is
    /// no data. `peak` keeps the highest sample of each span instead of the mean.
    pub fn points(&self, seconds: u32, max_points: usize, peak: bool) -> Vec<f32> {
        if seconds as usize <= FINE_SAMPLES {
            let samples: Vec<f32> = self.fine.iter().copied().collect();
            resample(&samples, self.fine_total, seconds as usize, max_points, peak)
        } else {
            let pick = |b: &Bucket| if peak { b.high } else { b.avg };
            let samples: Vec<f32> = self.coarse.iter().map(pick).collect();
            resample(&samples, self.coarse_total, (seconds / COARSE_SECONDS) as usize, max_points, peak)
        }
    }
}

/// Reduces a window of samples to at most `max_points`, grouping them when needed.
///
/// `total` is how many samples the series has ever received. Groups are fixed runs
/// of that count rather than being measured back from the newest sample, so a
/// finished group never changes and the graph slides instead of changing shape.
fn resample(samples: &[f32], total: u64, window: usize, max_points: usize, peak: bool) -> Vec<f32> {
    let Grouping { size: group, count, newest: newest_group } = Grouping::new(total, window, max_points);
    let first_held = total - samples.len() as u64;

    (0..count)
        .map(|index| {
            let mut span = Accumulator::default();
            if let Some(number) = newest_group.checked_sub(count - 1 - index) {
                for position in (number * group)..((number + 1) * group).min(total) {
                    if position >= first_held {
                        let value = samples[(position - first_held) as usize];
                        span.add(value, value, value);
                    }
                }
            }
            if peak { span.high.unwrap_or(f32::NAN) } else { span.avg().unwrap_or(f32::NAN) }
        })
        .collect()
}

/// How a window of samples is split into points.
struct Grouping {
    /// Samples per point.
    size: u64,
    /// Points across the window.
    count: u64,
    /// Number of the group holding the newest sample.
    newest: u64,
}

impl Grouping {
    fn new(total: u64, window: usize, max_points: usize) -> Self {
        let size = window.div_ceil(max_points.max(1)).max(1) as u64;
        let count = (window as u64 / size).max(2);
        Self { size, count, newest: total.saturating_sub(1) / size }
    }
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
        assert_eq!(stats.low, Some(10.0));
        assert_eq!(stats.high, Some(30.0));
        assert_eq!(stats.avg, Some(20.0));
    }

    #[test]
    fn gaps_are_ignored_by_stats() {
        let stats = filled(&[10.0, f32::NAN, 30.0]).stats(60);
        assert_eq!(stats.low, Some(10.0));
        assert_eq!(stats.high, Some(30.0));
        assert_eq!(stats.avg, Some(20.0));
    }

    #[test]
    fn missing_latest_reading_is_not_reported_as_current() {
        assert_eq!(filled(&[10.0, f32::NAN]).stats(60).now, None);
        assert_eq!(filled(&[f32::NAN]).stats(60).high, None);
    }

    #[test]
    fn long_windows_use_minute_buckets_and_keep_the_extremes() {
        let mut series = Series::default();
        for i in 0..7200 {
            series.push(match i {
                100 => 99.0,
                200 => 20.0,
                _ => 40.0,
            });
        }
        // The spikes are older than the fine ring, so only a bucket can remember them.
        let stats = series.stats(6 * 3600);
        assert_eq!(stats.high, Some(99.0));
        assert_eq!(stats.low, Some(20.0));
        assert!((stats.avg.unwrap() - 40.0).abs() < 0.1);
        assert_eq!(series.stats(3600).high, Some(40.0));
        assert_eq!(series.stats(3600).low, Some(40.0));
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

    /// Two samples is one group at this width, so everything moves along by one.
    fn assert_slides(window: u32, peak: bool) {
        let mut series = Series::default();
        let wave = |i: usize| ((i * 37) % 101) as f32;
        for i in 0..1000 {
            series.push(wave(i));
        }
        let before = series.points(window, 588, peak);
        series.push(wave(1000));
        series.push(wave(1001));
        let after = series.points(window, 588, peak);

        assert_eq!(before.len(), after.len());
        let settled = before.len() - 2;
        assert_eq!(&after[..settled], &before[1..=settled], "history must not change shape");
    }

    #[test]
    fn the_first_point_moves_on_by_one_as_the_graph_slides() {
        // 999 samples leave the newest group of two half full.
        let mut series = filled(&[1.0; 999]);
        let before = series.first_point(900, 588);
        series.push(1.0);
        assert_eq!(series.first_point(900, 588), before, "half a group is not a new point");
        series.push(1.0);
        assert_eq!(series.first_point(900, 588), before + 1);
    }

    #[test]
    fn long_windows_keep_their_shape_as_samples_arrive() {
        assert_slides(900, false);
        assert_slides(900, true);
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
