//! The two operators the cleanup pass applies.
//!
//! | Operator | Fixes |
//! |---|---|
//! | [`kuwahara`] | JPEG ringing and mosquito noise in flat regions |
//! | [`toggle_contrast`] | the soft transition band of a blurred edge |
//!
//! Both measure on luminance and both leave the image piecewise constant,
//! which is what the tracer's colour clustering needs. Neither is the
//! Photoshop answer: an unsharp mask is a high-pass boost, so it amplifies
//! noise and rings on both sides of every edge, and a colour clusterer reads
//! the rings as new colour bands. `CLEANUP_IMPLEMENTATION_PLAN.md` §9.2 has
//! the measurement.
//!
//! # Exact arithmetic
//!
//! Luminance here is Rec. 601 in fixed point, `299 r + 587 g + 114 b`, an
//! exact integer. The comparisons both operators make (which quadrant has
//! the lowest variance, which neighbour is darkest) are then exact, and the
//! optimised forms below reproduce the naive forms bit for bit. A float
//! version cannot promise that: the sum of squares over a 1024x1024 image
//! needs more than 53 bits.
//!
//! # Two invariants
//!
//! Toggle contrast selects a **whole pixel**, never a per-channel extreme.
//! A per-channel minimum would synthesise colours absent from the input, which
//! is exactly the halo behaviour that makes the unsharp mask lose.
//!
//! Kuwahara **averages every channel, alpha included**, but measures variance
//! on luminance only.

use vtracer::ColorImage;

/// One straight RGBA pixel.
pub(crate) type Pixel = [u8; 4];

/// Rec. 601 luminance of a pixel, times 1000, as an exact integer.
///
/// Same coefficients as [`crate::cleanup::estimate`], without the float.
pub(crate) fn luma_fixed(pixel: Pixel) -> u32 {
    let [r, g, b, _] = pixel;
    299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b)
}

/// An image as rows of pixels, with clamp-to-edge reads.
struct Pixels<'a> {
    data: &'a [Pixel],
    width: usize,
    height: usize,
}

impl<'a> Pixels<'a> {
    /// `None` when the buffer does not hold `width * height` pixels, in
    /// which case there is nothing sensible to filter.
    fn of(image: &'a ColorImage) -> Option<Self> {
        let data = image.pixels.as_chunks::<4>().0;
        (data.len() == image.width * image.height && image.width > 0 && image.height > 0).then_some(
            Self {
                data,
                width: image.width,
                height: image.height,
            },
        )
    }

    /// Row `y`, clamped into the image.
    fn row(&self, y: usize) -> &'a [Pixel] {
        let y = y.min(self.height - 1);
        self.data
            .get(y * self.width..(y + 1) * self.width)
            .unwrap_or(&[])
    }

    /// The pixel at `(row, column)`, or black outside the image.
    fn at(&self, (y, x): (usize, usize)) -> Pixel {
        self.row(y).get(x).copied().unwrap_or([0; 4])
    }
}

/// `index` clamped into `0..len`, as a `usize`.
fn clamp_index(index: i64, len: usize) -> usize {
    let last = i64::try_from(len).unwrap_or(i64::MAX).saturating_sub(1);
    usize::try_from(index.clamp(0, last.max(0))).unwrap_or(0)
}

/// How many output rows one unit of work covers. Each band recomputes a
/// margin of `radius` rows on either side, so a band is much taller than any
/// radius in use, and small enough that its working set stays in cache.
const BAND_ROWS: usize = 32;

/// Run `band` over the image in bands of [`BAND_ROWS`] rows, in parallel or
/// in sequence, and collect the output. Each band is a pure function of the
/// input rows it reads, so the two paths give the same bytes.
fn filter_in_bands(
    image: &ColorImage,
    parallel: bool,
    band: impl Fn(usize, &mut [Pixel]) + Sync,
) -> ColorImage {
    let Some(pixels) = Pixels::of(image) else {
        return image.clone();
    };
    let mut out = vec![[0_u8; 4]; pixels.data.len()];
    let band_len = BAND_ROWS * pixels.width;
    if parallel {
        use rayon::prelude::*;
        out.par_chunks_mut(band_len)
            .enumerate()
            .for_each(|(index, rows)| band(index * BAND_ROWS, rows));
    } else {
        for (index, rows) in out.chunks_mut(band_len).enumerate() {
            band(index * BAND_ROWS, rows);
        }
    }
    ColorImage {
        pixels: out.into_flattened(),
        width: image.width,
        height: image.height,
    }
}

// --- Kuwahara ---------------------------------------------------------------

/// Kuwahara filter: an edge-preserving smoother.
///
/// Around each pixel, four overlapping square quadrants of side `radius + 1`
/// share the centre. The output is the per-channel mean of the quadrant whose
/// luminance variance is lowest. Flat regions get averaged; an edge is never
/// straddled, because the quadrant on one side of it always has the lower
/// variance.
///
/// Each quadrant costs O(1): a sliding window along every row gives the sum
/// of `radius + 1` pixels at every position, and `radius + 1` of those rows
/// added together give the sum of a whole quadrant.
///
/// `radius` 0 is the identity. `parallel` splits the rows over the `rayon`
/// pool; the output is the same either way.
///
/// # Examples
///
/// ```
/// use vectorise::cleanup::filter::kuwahara;
/// use vtracer::ColorImage;
///
/// let flat = ColorImage { pixels: [90, 60, 30, 255].repeat(8 * 8), width: 8, height: 8 };
/// assert_eq!(kuwahara(&flat, 3, false).pixels, flat.pixels);
/// ```
#[must_use]
pub fn kuwahara(image: &ColorImage, radius: u8, parallel: bool) -> ColorImage {
    let radius = usize::from(radius);
    filter_in_bands(image, parallel, |y0, out| {
        if let Some(pixels) = Pixels::of(image) {
            kuwahara_band(&pixels, radius, y0, out);
        }
    })
}

/// Running sums over a box of pixels: luminance, its square, and each channel.
///
/// The bounds hold for any `u8` radius: a quadrant has at most 65 536
/// pixels, so the channel sums fit `u32` and the luminance sums fit `u64`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BoxSum {
    luma: u64,
    luma_squared: u64,
    channels: [u32; 4],
}

impl BoxSum {
    fn of(pixel: Pixel) -> Self {
        let luma = u64::from(luma_fixed(pixel));
        Self {
            luma,
            luma_squared: luma * luma,
            channels: pixel.map(u32::from),
        }
    }

    fn add(&mut self, other: Self) {
        self.luma += other.luma;
        self.luma_squared += other.luma_squared;
        for (sum, value) in self.channels.iter_mut().zip(other.channels) {
            *sum += value;
        }
    }

    fn remove(&mut self, other: Self) {
        self.luma -= other.luma;
        self.luma_squared -= other.luma_squared;
        for (sum, value) in self.channels.iter_mut().zip(other.channels) {
            *sum -= value;
        }
    }

    /// `count² × variance`, exact. Every quadrant has the same count, so this
    /// orders them the same way the variance does.
    fn variance_numerator(&self, count: u64) -> u128 {
        u128::from(count) * u128::from(self.luma_squared)
            - u128::from(self.luma) * u128::from(self.luma)
    }

    /// The per-channel mean over `count` pixels, rounded half up.
    ///
    /// `u32` arithmetic on purpose: the division is the hot spot of the whole
    /// filter, and the sums fit (a quadrant has at most 65 536 pixels).
    fn mean(&self, count: u32) -> Pixel {
        self.channels.map(|sum| {
            let rounded = (2 * sum + count) / (2 * count.max(1));
            u8::try_from(rounded).unwrap_or(u8::MAX)
        })
    }
}

/// The sum of `radius + 1` consecutive pixels at every position along `row`,
/// clamped at both ends, in `out`, which holds `width + radius` entries.
/// `terms` is scratch space, reused across rows.
///
/// Entry `p` covers the pixels `p - radius ..= p`. Quadrant `(qx, _)` of the
/// pixel at `x` therefore starts at entry `x` for `qx = -1` and at entry
/// `x + radius` for `qx = 0`.
fn box_sums_along(row: &[Pixel], radius: usize, terms: &mut Vec<BoxSum>, out: &mut [BoxSum]) {
    terms.clear();
    terms.extend(row.iter().map(|&pixel| BoxSum::of(pixel)));
    let term = |index: i64| {
        terms
            .get(clamp_index(index, terms.len()))
            .copied()
            .unwrap_or_default()
    };
    let radius = i64::try_from(radius).unwrap_or(i64::MAX);
    let mut sum = BoxSum::default();
    for index in -radius..=0 {
        sum.add(term(index));
    }
    for (position, slot) in out.iter_mut().enumerate() {
        let position = i64::try_from(position).unwrap_or(i64::MAX);
        if position > 0 {
            sum.add(term(position));
            sum.remove(term(position - radius - 1));
        }
        *slot = sum;
    }
}

/// A ring of the last few rows of a streaming computation, addressed by the
/// absolute row number. Keeps a band's working set at a handful of rows.
struct Ring<T> {
    rows: Vec<Vec<T>>,
}

impl<T: Copy + Default> Ring<T> {
    fn new(rows: usize, width: usize) -> Self {
        Self {
            rows: vec![vec![T::default(); width]; rows.max(1)],
        }
    }

    fn get(&self, row: usize) -> &[T] {
        self.rows
            .get(row % self.rows.len())
            .map_or(&[][..], Vec::as_slice)
    }

    fn get_mut(&mut self, row: usize) -> &mut [T] {
        let len = self.rows.len();
        self.rows
            .get_mut(row % len)
            .map_or(&mut [][..], Vec::as_mut_slice)
    }
}

/// The Kuwahara output for rows `y0 ..` of the image, into `out`.
///
/// Streams down the band: local row `t` of the band's source rows (margins
/// included, clamped at the image edges) has its row sums computed once,
/// and stack `t`, the sum of rows `t ..= t + radius`, is the previous stack
/// with one row added and one removed. Output row `i` sits at local row
/// `i + radius`; its upper quadrants are stack `i`, its lower ones stack
/// `i + radius`. Two small rings hold exactly the rows still needed.
fn kuwahara_band(pixels: &Pixels<'_>, radius: usize, y0: usize, out: &mut [Pixel]) {
    let width = pixels.width;
    let count = u32::try_from((radius + 1) * (radius + 1)).unwrap_or(u32::MAX);
    let band_rows = out.len() / width;
    let first = i64::try_from(y0).unwrap_or(i64::MAX) - i64::try_from(radius).unwrap_or(i64::MAX);
    let source_row = |local: usize| {
        pixels.row(clamp_index(
            first + i64::try_from(local).unwrap_or(i64::MAX),
            pixels.height,
        ))
    };

    let mut row_sums: Ring<BoxSum> = Ring::new(radius + 2, width + radius);
    let mut stacks: Ring<BoxSum> = Ring::new(radius + 1, width + radius);
    let mut terms = Vec::with_capacity(width);

    // Stack 0 from scratch.
    for local in 0..=radius {
        box_sums_along(
            source_row(local),
            radius,
            &mut terms,
            row_sums.get_mut(local),
        );
    }
    let mut stack = vec![BoxSum::default(); width + radius];
    for local in 0..=radius {
        for (acc, term) in stack.iter_mut().zip(row_sums.get(local)) {
            acc.add(*term);
        }
    }
    stacks.get_mut(0).copy_from_slice(&stack);

    for t in 0..band_rows + radius {
        if t > 0 {
            box_sums_along(
                source_row(t + radius),
                radius,
                &mut terms,
                row_sums.get_mut(t + radius),
            );
            let (added, removed) = (row_sums.get(t + radius), row_sums.get(t - 1));
            for ((acc, added), removed) in stack.iter_mut().zip(added).zip(removed) {
                acc.add(*added);
                acc.remove(*removed);
            }
            stacks.get_mut(t).copy_from_slice(&stack);
        }
        if t < radius {
            continue;
        }
        let index = t - radius;
        let (upper, lower) = (stacks.get(index), stacks.get(t));
        let quadrants = upper
            .iter()
            .zip(upper.iter().skip(radius))
            .zip(lower.iter().zip(lower.iter().skip(radius)));
        let out_row = out
            .get_mut(index * width..(index + 1) * width)
            .unwrap_or(&mut []);
        for (pixel, ((top_left, top_right), (bottom_left, bottom_right))) in
            out_row.iter_mut().zip(quadrants)
        {
            *pixel = lowest_variance([top_left, top_right, bottom_left, bottom_right], count);
        }
    }
}

/// The mean of the first quadrant with the lowest variance, in the order
/// `(-1, -1)`, `(0, -1)`, `(-1, 0)`, `(0, 0)`.
fn lowest_variance(quadrants: [&BoxSum; 4], count: u32) -> Pixel {
    let mut best: Option<(u128, &BoxSum)> = None;
    for quadrant in quadrants {
        let variance = quadrant.variance_numerator(u64::from(count));
        if best.is_none_or(|(lowest, _)| variance < lowest) {
            best = Some((variance, quadrant));
        }
    }
    best.map_or([0; 4], |(_, quadrant)| quadrant.mean(count))
}

// --- toggle contrast --------------------------------------------------------

/// Morphological toggle contrast: snap every pixel to the darkest or the
/// brightest pixel of its neighbourhood, whichever is closer in luminance.
///
/// The square window has side `2 × radius + 1`. The result is always a pixel
/// that was already in the input, all four channels of it, so the operator
/// cannot create a halo. Ties go to the darker side.
///
/// The darkest and brightest neighbours are found with the van Herk and
/// Gil-Werman running extreme, separably: once along every row, then along
/// every column of the row results. That is O(1) per pixel per axis whatever
/// the radius. Among equal luminances the first pixel in row-major order
/// wins, unless the centre itself ties, in which case the centre wins.
///
/// `radius` 0 is the identity. `parallel` splits the rows over the `rayon`
/// pool; the output is the same either way.
///
/// # Examples
///
/// ```
/// use vectorise::cleanup::filter::toggle_contrast;
/// use vtracer::ColorImage;
///
/// // A grey between black and white snaps to whichever is closer: 100 is
/// // nearer black, 200 nearer white.
/// let pixels = [[0, 0, 0, 255], [100, 100, 100, 255], [200, 200, 200, 255], [255, 255, 255, 255]];
/// let ramp = ColorImage { pixels: pixels.concat(), width: 4, height: 1 };
/// let snapped = toggle_contrast(&ramp, 1, false);
/// assert_eq!(
///     snapped.pixels,
///     [[0, 0, 0, 255], [0, 0, 0, 255], [255, 255, 255, 255], [255, 255, 255, 255]].concat()
/// );
/// ```
#[must_use]
pub fn toggle_contrast(image: &ColorImage, radius: u8, parallel: bool) -> ColorImage {
    let radius = usize::from(radius);
    filter_in_bands(image, parallel, |y0, out| {
        if let Some(pixels) = Pixels::of(image) {
            toggle_band(&pixels, radius, y0, out);
        }
    })
}

/// Bits of a packed key given to each of the row and the column.
const COORD_BITS: u32 = 23;
/// The largest coordinate a key can carry.
const COORD_MASK: u64 = (1 << COORD_BITS) - 1;

/// A pixel packed as `(luminance, row, column)`, most significant first.
///
/// The minimum over a set of keys is then the darkest pixel, and among
/// equally dark ones the first in row-major order, which is the naive
/// operator's scan order. Every key is unique, so no tie ever reaches the
/// running extreme and it can use a plain `min`. The brightest pixel uses
/// the same trick with both coordinates reflected, so a plain `max` picks
/// the first in scan order among the brightest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key(u64);

impl Key {
    fn dark(pixel: Pixel, y: usize, x: usize) -> Self {
        Self::pack(luma_fixed(pixel), coordinate(y), coordinate(x))
    }

    fn bright(pixel: Pixel, y: usize, x: usize) -> Self {
        Self::pack(
            luma_fixed(pixel),
            COORD_MASK - coordinate(y),
            COORD_MASK - coordinate(x),
        )
    }

    fn pack(luma: u32, y: u64, x: u64) -> Self {
        Self((u64::from(luma) << (2 * COORD_BITS)) | (y << COORD_BITS) | x)
    }

    fn luma(self) -> u32 {
        u32::try_from(self.0 >> (2 * COORD_BITS)).unwrap_or(u32::MAX)
    }

    /// The `(row, column)` a dark key names.
    fn dark_position(self) -> (usize, usize) {
        (
            usize::try_from((self.0 >> COORD_BITS) & COORD_MASK).unwrap_or(0),
            usize::try_from(self.0 & COORD_MASK).unwrap_or(0),
        )
    }

    /// The `(row, column)` a bright key names.
    fn bright_position(self) -> (usize, usize) {
        let (y, x) = self.dark_position();
        (
            usize::try_from(COORD_MASK).unwrap_or(usize::MAX) - y,
            usize::try_from(COORD_MASK).unwrap_or(usize::MAX) - x,
        )
    }
}

/// A coordinate as key bits. Images wider or taller than 2^23 are clamped;
/// at 8 million pixels a side the image would not fit in memory anyway.
fn coordinate(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX).min(COORD_MASK)
}

/// `min` or `max`, chosen once per pass and inlined into every loop.
trait Extreme: Copy {
    fn pick(self, a: Key, b: Key) -> Key;
}

#[derive(Clone, Copy)]
struct Lowest;

#[derive(Clone, Copy)]
struct Highest;

impl Extreme for Lowest {
    fn pick(self, a: Key, b: Key) -> Key {
        a.min(b)
    }
}

impl Extreme for Highest {
    fn pick(self, a: Key, b: Key) -> Key {
        a.max(b)
    }
}

/// Scratch space for [`running_extreme`], reused across rows.
#[derive(Default)]
struct Tables {
    prefix: Vec<Key>,
    suffix: Vec<Key>,
}

/// The van Herk and Gil-Werman running extreme along one sequence, into
/// `out`.
///
/// The sequence is cut into blocks of `2 × radius + 1`. `prefix[i]` is the
/// extreme from the start of `i`'s block up to `i`, `suffix[i]` from `i` to
/// the end of its block. Any window of the block length then spans the tail
/// of one block and the head of the next, and its extreme is the better of
/// one suffix entry and one prefix entry: three comparisons per position.
fn running_extreme<E: Extreme>(
    values: &[Key],
    radius: usize,
    extreme: E,
    tables: &mut Tables,
    out: &mut [Key],
) {
    let block = 2 * radius + 1;
    let len = values.len();

    tables.prefix.clear();
    for (index, &value) in values.iter().enumerate() {
        let keep = match tables.prefix.last() {
            Some(&sofar) if !index.is_multiple_of(block) => extreme.pick(sofar, value),
            _ => value,
        };
        tables.prefix.push(keep);
    }

    tables.suffix.clear();
    tables.suffix.resize(len, Key(0));
    let mut next: Option<Key> = None;
    for (index, (slot, &value)) in tables.suffix.iter_mut().zip(values).enumerate().rev() {
        let keep = match next {
            Some(later) if !(index + 1).is_multiple_of(block) => extreme.pick(later, value),
            _ => value,
        };
        *slot = keep;
        next = Some(keep);
    }

    for (index, slot) in out.iter_mut().enumerate() {
        let (a, b) = (index.saturating_sub(radius), (index + radius).min(len - 1));
        let head = tables.suffix.get(a).copied().unwrap_or(Key(0));
        let tail = tables.prefix.get(b).copied().unwrap_or(Key(0));
        *slot = window_extreme(head, tail, a, b, block, extreme);
    }
}

/// The extreme over the window `a ..= b`, given the suffix entry at `a`
/// (`head`) and the prefix entry at `b` (`tail`). The window is at most one
/// block long.
fn window_extreme<E: Extreme>(
    head: Key,
    tail: Key,
    a: usize,
    b: usize,
    block: usize,
    extreme: E,
) -> Key {
    if a / block == b / block {
        // One block. A window that starts on a block boundary is covered by
        // the prefix entry at its end; one that does not is cut short by the
        // end of the sequence and covered by the suffix entry at its start.
        if a.is_multiple_of(block) { tail } else { head }
    } else {
        extreme.pick(head, tail)
    }
}

/// Rows of keys, `width` entries each, stored flat.
struct Rows {
    data: Vec<Key>,
    width: usize,
}

impl Rows {
    fn blank(rows: usize, width: usize) -> Self {
        Self {
            data: vec![Key(0); rows * width],
            width,
        }
    }

    const fn len(&self) -> usize {
        self.data.len() / self.width
    }

    fn row(&self, index: usize) -> &[Key] {
        self.data
            .get(index * self.width..(index + 1) * self.width)
            .unwrap_or(&[])
    }

    fn rows_mut(&mut self) -> std::slice::ChunksExactMut<'_, Key> {
        self.data.chunks_exact_mut(self.width)
    }
}

/// [`running_extreme`] along every row of `rows`.
fn extremes_along<E: Extreme>(rows: &Rows, radius: usize, extreme: E) -> Rows {
    let mut out = Rows::blank(rows.len(), rows.width);
    let mut tables = Tables::default();
    for (index, slot) in out.rows_mut().enumerate() {
        running_extreme(rows.row(index), radius, extreme, &mut tables, slot);
    }
    out
}

/// The prefix and suffix tables of [`running_extreme`] down the columns of
/// `rows`: the same recurrences, each step applied to a whole row at once.
fn tables_down<E: Extreme>(rows: &Rows, radius: usize, extreme: E) -> (Rows, Rows) {
    let block = 2 * radius + 1;
    let (len, width) = (rows.len(), rows.width);

    let mut prefix = Rows::blank(len, width);
    let mut previous = vec![Key(0); width];
    for (index, slot) in prefix.rows_mut().enumerate() {
        let row = rows.row(index);
        if index.is_multiple_of(block) {
            slot.copy_from_slice(row);
        } else {
            for ((out, &value), &sofar) in slot.iter_mut().zip(row).zip(&previous) {
                *out = extreme.pick(sofar, value);
            }
        }
        previous.copy_from_slice(slot);
    }

    let mut suffix = Rows::blank(len, width);
    let mut later = vec![Key(0); width];
    for (index, slot) in suffix.rows_mut().enumerate().rev() {
        let row = rows.row(index);
        if (index + 1).is_multiple_of(block) || index + 1 == len {
            slot.copy_from_slice(row);
        } else {
            for ((out, &value), &later) in slot.iter_mut().zip(row).zip(&later) {
                *out = extreme.pick(later, value);
            }
        }
        later.copy_from_slice(slot);
    }

    (prefix, suffix)
}

/// The toggle contrast output for rows `y0 ..` of the image, into `out`.
fn toggle_band(pixels: &Pixels<'_>, radius: usize, y0: usize, out: &mut [Pixel]) {
    let width = pixels.width;
    let band_rows = out.len() / width;
    // Source rows this band reads. Unlike Kuwahara's clamped margin, a window
    // that reaches past the image is simply cut short there, so the rows are
    // the real ones only.
    let first = y0.saturating_sub(radius);
    let last = (y0 + band_rows + radius).min(pixels.height);
    let mut dark = Rows::blank(last - first, width);
    let mut bright = Rows::blank(last - first, width);
    for ((dark_row, bright_row), y) in dark.rows_mut().zip(bright.rows_mut()).zip(first..last) {
        for (x, ((dark, bright), &pixel)) in dark_row
            .iter_mut()
            .zip(bright_row.iter_mut())
            .zip(pixels.row(y))
            .enumerate()
        {
            *dark = Key::dark(pixel, y, x);
            *bright = Key::bright(pixel, y, x);
        }
    }

    let (dark_prefix, dark_suffix) =
        tables_down(&extremes_along(&dark, radius, Lowest), radius, Lowest);
    let (bright_prefix, bright_suffix) =
        tables_down(&extremes_along(&bright, radius, Highest), radius, Highest);

    let block = 2 * radius + 1;
    let len = dark.len();
    for (index, out_row) in out.chunks_exact_mut(width).enumerate() {
        let local = y0 + index - first;
        let (a, b) = (local.saturating_sub(radius), (local + radius).min(len - 1));
        let darkest = dark_suffix
            .row(a)
            .iter()
            .zip(dark_prefix.row(b))
            .map(|(&head, &tail)| window_extreme(head, tail, a, b, block, Lowest));
        let brightest = bright_suffix
            .row(a)
            .iter()
            .zip(bright_prefix.row(b))
            .map(|(&head, &tail)| window_extreme(head, tail, a, b, block, Highest));
        let centres = pixels.row(y0 + index);
        for (pixel, (&centre, (low, high))) in out_row
            .iter_mut()
            .zip(centres.iter().zip(darkest.zip(brightest)))
        {
            *pixel = snap(pixels, centre, low, high);
        }
    }
}

/// `centre` snapped to the pixel `low` names or the one `high` names,
/// whichever is closer in luminance, the darker one on a tie. A neighbour
/// that only ties the centre does not replace it: the centre is its own
/// darkest or brightest pixel then.
fn snap(pixels: &Pixels<'_>, centre: Pixel, low: Key, high: Key) -> Pixel {
    let luma = luma_fixed(centre);
    let (low_luma, high_luma) = (low.luma(), high.luma());
    let down = luma - low_luma;
    let up = high_luma - luma;
    if down <= up {
        if low_luma == luma {
            centre
        } else {
            pixels.at(low.dark_position())
        }
    } else if high_luma == luma {
        centre
    } else {
        pixels.at(high.bright_position())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use std::collections::BTreeSet;

    use vtracer::ColorImage;

    use super::{Pixel, kuwahara, toggle_contrast};
    use crate::cleanup::degraded::{Degradation, Fixture, degrade, from_rgba, to_rgba};

    /// The naive forms of both operators, ported line for line from
    /// `CLEANUP_IMPLEMENTATION_PLAN.md` §10.4 and §10.5 with the crate's
    /// exact integer luminance. They are the oracle the optimised forms are
    /// held to, bit for bit.
    mod naive {
        use vtracer::ColorImage;

        use super::super::{Pixel, clamp_index, luma_fixed};

        fn at(image: &ColorImage, x: i64, y: i64) -> Pixel {
            let x = clamp_index(x, image.width);
            let y = clamp_index(y, image.height);
            let i = (y * image.width + x) * 4;
            [
                image.pixels[i],
                image.pixels[i + 1],
                image.pixels[i + 2],
                image.pixels[i + 3],
            ]
        }

        fn each_pixel(image: &ColorImage, f: impl Fn(i64, i64) -> Pixel) -> ColorImage {
            let mut pixels = Vec::with_capacity(image.pixels.len());
            for y in 0..i64::try_from(image.height).expect("fits") {
                for x in 0..i64::try_from(image.width).expect("fits") {
                    pixels.extend_from_slice(&f(x, y));
                }
            }
            ColorImage {
                pixels,
                width: image.width,
                height: image.height,
            }
        }

        pub(super) fn kuwahara(image: &ColorImage, radius: i64) -> ColorImage {
            let quadrants: [(i64, i64); 4] = [(-1, -1), (0, -1), (-1, 0), (0, 0)];
            each_pixel(image, |x, y| {
                let mut best: Option<(u128, Pixel)> = None;
                for (qx, qy) in quadrants {
                    let (x0, y0) = (x + qx * radius, y + qy * radius);
                    let (mut sum, mut sum_l, mut sum_l2, mut n) = ([0_u64; 4], 0_u64, 0_u64, 0_u64);
                    for dy in 0..=radius {
                        for dx in 0..=radius {
                            let s = at(image, x0 + dx, y0 + dy);
                            let l = u64::from(luma_fixed(s));
                            sum_l += l;
                            sum_l2 += l * l;
                            for c in 0..4 {
                                sum[c] += u64::from(s[c]);
                            }
                            n += 1;
                        }
                    }
                    let var = u128::from(n) * u128::from(sum_l2) - u128::from(sum_l).pow(2);
                    if best.is_none_or(|(lowest, _)| var < lowest) {
                        let mut mean = [0_u8; 4];
                        for c in 0..4 {
                            mean[c] =
                                u8::try_from((2 * sum[c] + n) / (2 * n)).expect("a mean of bytes");
                        }
                        best = Some((var, mean));
                    }
                }
                best.expect("four quadrants").1
            })
        }

        pub(super) fn toggle_contrast(image: &ColorImage, radius: i64) -> ColorImage {
            each_pixel(image, |x, y| {
                let centre = at(image, x, y);
                let cl = luma_fixed(centre);
                let (mut lo, mut hi) = (centre, centre);
                let (mut lo_l, mut hi_l) = (cl, cl);
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let s = at(image, x + dx, y + dy);
                        let l = luma_fixed(s);
                        if l < lo_l {
                            lo_l = l;
                            lo = s;
                        }
                        if l > hi_l {
                            hi_l = l;
                            hi = s;
                        }
                    }
                }
                if (cl - lo_l) <= (hi_l - cl) { lo } else { hi }
            })
        }
    }

    /// A 64x64 opaque greyscale image whose value at `(x, y)` is `f(x, y)`.
    fn grey(f: impl Fn(usize, usize) -> u8) -> ColorImage {
        let mut pixels = Vec::with_capacity(64 * 64 * 4);
        for y in 0..64 {
            for x in 0..64 {
                let value = f(x, y);
                pixels.extend_from_slice(&[value, value, value, 255]);
            }
        }
        ColorImage {
            pixels,
            width: 64,
            height: 64,
        }
    }

    fn hard_step() -> ColorImage {
        grey(|x, _| if x < 32 { 0 } else { 255 })
    }

    fn pixel(image: &ColorImage, x: usize, y: usize) -> Pixel {
        let base = (y * image.width + x) * 4;
        [
            image.pixels[base],
            image.pixels[base + 1],
            image.pixels[base + 2],
            image.pixels[base + 3],
        ]
    }

    fn colours(image: &ColorImage) -> BTreeSet<Pixel> {
        image.pixels.as_chunks::<4>().0.iter().copied().collect()
    }

    /// A deterministic pseudo-random sequence.
    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *seed >> 33
    }

    /// A random RGBA image with `levels` distinct values per channel, so ties
    /// in luminance are common and the tie-break rules get exercised.
    fn random_image(seed: u64, width: usize, height: usize, levels: u64) -> ColorImage {
        let mut seed = seed;
        let pixels = (0..width * height * 4)
            .map(|_| u8::try_from(lcg(&mut seed) % levels * (255 / (levels - 1))).unwrap_or(255))
            .collect();
        ColorImage {
            pixels,
            width,
            height,
        }
    }

    // --- Kuwahara ------------------------------------------------------------

    #[test]
    fn kuwahara_of_flat_image_is_identity() {
        let flat = grey(|_, _| 77);
        for radius in 1..=3 {
            assert_eq!(
                kuwahara(&flat, radius, false).pixels,
                flat.pixels,
                "r{radius}"
            );
        }
    }

    #[test]
    fn kuwahara_preserves_a_hard_edge_position() {
        let filtered = kuwahara(&hard_step(), 3, false);
        for y in 0..64 {
            assert_eq!(pixel(&filtered, 31, y), [0, 0, 0, 255], "row {y}");
            assert_eq!(pixel(&filtered, 32, y), [255, 255, 255, 255], "row {y}");
        }
        assert_eq!(
            filtered.pixels,
            hard_step().pixels,
            "the whole step survives"
        );
    }

    #[test]
    fn kuwahara_removes_salt_and_pepper() {
        // Specks on an 8-pixel lattice. Every neighbour of a speck has a 3x3
        // quadrant that avoids it, with zero variance, so it comes back
        // exactly flat. The speck itself sits in all four of its own
        // quadrants and is averaged into them: a 127-level outlier becomes a
        // 14-level one, nine times smaller.
        let speckled = grey(|x, y| match (x % 8, y % 8) {
            (3, 3) => 255,
            (7, 5) => 0,
            _ => 128,
        });
        let filtered = kuwahara(&speckled, 2, false);
        for y in 0..64 {
            for x in 0..64 {
                let value = pixel(&filtered, x, y)[0];
                if matches!((x % 8, y % 8), (3, 3) | (7, 5)) {
                    assert!(value.abs_diff(128) <= 15, "speck at {x},{y} is {value}");
                } else {
                    assert_eq!(value, 128, "at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn kuwahara_radius_zero_is_identity() {
        let image = degrade(Fixture::Logo, Degradation::Jpeg30);
        assert_eq!(kuwahara(&image, 0, false).pixels, image.pixels);
    }

    #[test]
    fn kuwahara_averages_alpha_with_the_other_channels() {
        // A flat colour whose alpha alternates: the mean of the winning
        // quadrant is an intermediate alpha, computed like any other channel.
        let mut image = grey(|_, _| 100);
        for (index, pixel) in image.pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pixel[3] = if index % 2 == 0 { 0 } else { 255 };
        }
        let filtered = kuwahara(&image, 1, false);
        let alphas: BTreeSet<u8> = filtered
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .collect();
        assert!(
            alphas.iter().all(|a| (0..=255).contains(a)) && alphas.len() > 1,
            "{alphas:?}"
        );
        assert!(
            filtered
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[..3] == [100, 100, 100]),
            "colour is untouched"
        );
    }

    // --- toggle contrast -----------------------------------------------------

    #[test]
    fn toggle_of_flat_image_is_identity() {
        let flat = grey(|_, _| 77);
        for radius in 1..=3 {
            assert_eq!(
                toggle_contrast(&flat, radius, false).pixels,
                flat.pixels,
                "r{radius}"
            );
        }
    }

    #[test]
    fn toggle_snaps_a_ramp_to_a_step() {
        // A five-pixel ramp from 0 to 255 across x = 30..=34.
        let ramp = grey(|x, _| match x {
            0..=29 => 0,
            30..=34 => u8::try_from((x - 30) * 255 / 4).expect("fits"),
            _ => 255,
        });
        let snapped = toggle_contrast(&ramp, 3, false);
        for y in 0..64 {
            let row: Vec<u8> = (0..64).map(|x| pixel(&snapped, x, y)[0]).collect();
            assert!(
                row.iter().all(|&v| v == 0 || v == 255),
                "row {y} is two flat runs: {row:?}"
            );
            assert!(row.windows(2).all(|w| w[0] <= w[1]), "row {y}: {row:?}");
        }
    }

    #[test]
    fn toggle_introduces_no_new_colors() {
        for (fixture, degradation) in [
            (Fixture::Logo, Degradation::Blur3),
            (Fixture::Star, Degradation::Soft),
            (Fixture::Disc, Degradation::Jpeg30),
        ] {
            let input = degrade(fixture, degradation);
            let before = colours(&input);
            for radius in 1..=3 {
                let after = colours(&toggle_contrast(&input, radius, false));
                assert!(
                    after.is_subset(&before),
                    "{} {} r{radius}: {} new colours",
                    fixture.name(),
                    degradation.name(),
                    after.difference(&before).count()
                );
            }
        }
    }

    #[test]
    fn toggle_preserves_edge_position_within_one_pixel() {
        let blurred = from_rgba(&image::imageops::blur(&to_rgba(&hard_step()), 2.0));
        let snapped = toggle_contrast(&blurred, 3, false);
        for y in 0..64 {
            let first_bright = (0..64)
                .find(|&x| pixel(&snapped, x, y)[0] > 127)
                .expect("a bright side");
            assert!(
                (31..=33).contains(&first_bright),
                "row {y}: the step moved to {first_bright}"
            );
        }
    }

    #[test]
    fn toggle_radius_zero_is_identity() {
        let image = degrade(Fixture::Logo, Degradation::Blur3);
        assert_eq!(toggle_contrast(&image, 0, false).pixels, image.pixels);
    }

    #[test]
    fn toggle_ties_go_to_the_darker_side() {
        // 0 | 128 | 255 with radius 1: the middle is equidistant and snaps down.
        let image = ColorImage {
            pixels: [[0, 0, 0, 255], [128, 128, 128, 255], [255, 255, 255, 255]].concat(),
            width: 3,
            height: 1,
        };
        // 128 - 0 = 128 > 255 - 128 = 127: the brighter side is closer here,
        // so use 127 for the exact tie instead.
        let mut tie = image.clone();
        tie.pixels[4..7].copy_from_slice(&[127, 127, 127]);
        // luma(127) - luma(0) = 127 000; luma(255) - luma(127) = 128 000.
        assert_eq!(
            pixel(&toggle_contrast(&tie, 1, false), 1, 0),
            [0, 0, 0, 255]
        );
        assert_eq!(
            pixel(&toggle_contrast(&image, 1, false), 1, 0),
            [255, 255, 255, 255]
        );
    }

    // --- both --------------------------------------------------------------------

    #[test]
    fn both_filters_preserve_dimensions_and_alpha() {
        let input = degrade(Fixture::Badge, Degradation::Blur15);
        for (name, output) in [
            ("kuwahara", kuwahara(&input, 3, false)),
            ("toggle", toggle_contrast(&input, 2, false)),
        ] {
            assert_eq!((output.width, output.height), (256, 256), "{name}");
            assert_eq!(output.pixels.len(), input.pixels.len(), "{name}");
            assert!(
                output.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
                "{name}: an opaque input stays opaque"
            );
        }
    }

    #[test]
    fn both_filters_are_deterministic() {
        let input = degrade(Fixture::Star, Degradation::Blur15Jpeg50);
        assert_eq!(
            kuwahara(&input, 3, false).pixels,
            kuwahara(&input, 3, false).pixels
        );
        assert_eq!(
            toggle_contrast(&input, 2, false).pixels,
            toggle_contrast(&input, 2, false).pixels
        );
    }

    #[test]
    fn filters_handle_1x1_and_1xn_images_without_panic() {
        for (width, height) in [(1, 1), (1, 17), (17, 1), (2, 2)] {
            let image = random_image(3, width, height, 4);
            for radius in [0, 1, 3, 8] {
                let smoothed = kuwahara(&image, radius, false);
                let snapped = toggle_contrast(&image, radius, false);
                assert_eq!((smoothed.width, smoothed.height), (width, height));
                assert_eq!((snapped.width, snapped.height), (width, height));
                assert_eq!(smoothed.pixels.len(), width * height * 4);
                assert!(colours(&snapped).is_subset(&colours(&image)));
            }
        }
    }

    #[test]
    fn filters_leave_a_malformed_buffer_alone() {
        let short = ColorImage {
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
            width: 3,
            height: 1,
        };
        assert_eq!(kuwahara(&short, 1, false).pixels, short.pixels);
        assert_eq!(toggle_contrast(&short, 1, false).pixels, short.pixels);
        let empty = ColorImage {
            pixels: Vec::new(),
            width: 0,
            height: 0,
        };
        assert_eq!(kuwahara(&empty, 1, false).pixels, Vec::<u8>::new());
        assert_eq!(toggle_contrast(&empty, 1, false).pixels, Vec::<u8>::new());
    }

    /// Inputs that exercise every tie-break: fixtures, a three-level image
    /// full of equal luminances, a full-range one, and awkward shapes.
    fn oracle_inputs() -> Vec<ColorImage> {
        vec![
            degrade(Fixture::Logo, Degradation::Blur3),
            degrade(Fixture::Badge, Degradation::Jpeg30),
            degrade(Fixture::Star, Degradation::Soft),
            random_image(21, 70, 70, 3),
            random_image(22, 64, 130, 2),
            random_image(23, 131, 9, 256),
            random_image(24, 1, 200, 5),
            random_image(25, 200, 1, 5),
        ]
    }

    #[test]
    fn optimised_kuwahara_matches_the_naive_oracle_exactly() {
        for input in oracle_inputs() {
            for radius in [1, 2, 3, 4, 7] {
                assert_eq!(
                    kuwahara(&input, radius, false).pixels,
                    naive::kuwahara(&input, i64::from(radius)).pixels,
                    "r{radius} {}x{}",
                    input.width,
                    input.height
                );
            }
        }
    }

    #[test]
    fn optimised_toggle_matches_the_naive_oracle_exactly() {
        for input in oracle_inputs() {
            for radius in [1, 2, 3, 4, 7] {
                assert_eq!(
                    toggle_contrast(&input, radius, false).pixels,
                    naive::toggle_contrast(&input, i64::from(radius)).pixels,
                    "r{radius} {}x{}",
                    input.width,
                    input.height
                );
            }
        }
    }

    #[test]
    fn parallel_and_sequential_paths_produce_identical_output() {
        let inputs = [
            degrade(Fixture::Logo, Degradation::Blur3),
            random_image(11, 97, 131, 3),
            random_image(12, 5, 300, 256),
        ];
        for input in &inputs {
            for radius in [1, 2, 3, 5] {
                assert_eq!(
                    kuwahara(input, radius, true).pixels,
                    kuwahara(input, radius, false).pixels,
                    "kuwahara r{radius} {}x{}",
                    input.width,
                    input.height
                );
                assert_eq!(
                    toggle_contrast(input, radius, true).pixels,
                    toggle_contrast(input, radius, false).pixels,
                    "toggle r{radius} {}x{}",
                    input.width,
                    input.height
                );
            }
        }
    }
}
