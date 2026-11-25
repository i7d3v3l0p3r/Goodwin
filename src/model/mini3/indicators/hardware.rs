use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::{Frame, RING_PIXELS, SEGMENTS};

type RingFrame = [u8; RING_PIXELS * 3];
const CLOCK_READBACK: Duration = Duration::from_millis(80);
const CLOCK_SETTLE: Duration = Duration::from_millis(40);

pub(super) struct Hardware {
    ring: PathBuf,
    segments: Vec<Segment>,
}

impl Hardware {
    pub(super) fn detect() -> Option<Self> {
        let leds = Path::new("/sys/class/leds");
        let graphics = Path::new("/sys/class/graphics/fb0");
        if !cfg!(target_os = "linux")
            || !leds.join("bergamot:red:logo0").exists()
            || !attribute_is(graphics, "name", "logo")
            || !attribute_is(graphics, "virtual_size", "6,1")
            || !attribute_is(graphics, "bits_per_pixel", "24")
            || !attribute_is(graphics, "stride", "18")
        {
            return None;
        }

        let names = (0..4)
            .flat_map(|digit| {
                (b'A'..=b'G').map(move |letter| format!("{}{digit}", char::from(letter)))
            })
            .chain(["DEGREE", "DH", "DL", "DM"].map(str::to_owned));
        let segments = names
            .map(|name| {
                let path = leds.join(format!("bergamot:white:clock{name}"));
                Some(Segment {
                    path: path.join("brightness"),
                    max: brightness(&path.join("max_brightness"))?,
                })
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();

        Some(Self {
            ring: PathBuf::from("/dev/fb0"),
            segments,
        })
    }

    pub(super) fn begin(&self) -> Lights<'_> {
        Lights {
            ring: Ring::open(&self.ring).ok(),
            clock: Clock::open(&self.segments),
        }
    }
}

fn attribute_is(directory: &Path, name: &str, expected: &str) -> bool {
    fs::read_to_string(directory.join(name)).is_ok_and(|value| value.trim() == expected)
}

pub(super) struct Lights<'a> {
    ring: Option<Ring>,
    clock: Option<Clock<'a>>,
}

impl Lights<'_> {
    pub(super) fn show(&mut self, frame: &Frame) {
        if let Some(ring) = &mut self.ring
            && !ring.show(&frame.ring)
        {
            // Another service has taken over the Station's indicators.
            self.ring = None;
            self.clock = None;
        }
        if let Some(clock) = &mut self.clock
            && !clock.show(&frame.clock, Instant::now())
        {
            self.clock = None;
        }
    }
}

struct Ring {
    file: File,
    before: RingFrame,
    last: RingFrame,
}

impl Ring {
    fn open(path: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let before = read_ring(&mut file)?;
        Ok(Self {
            file,
            before,
            last: before,
        })
    }

    fn show(&mut self, frame: &RingFrame) -> bool {
        let Ok(current) = read_ring(&mut self.file) else {
            return false;
        };
        if current != self.last {
            // A firmware redraw can clear the ring even when its animation is idle.
            if current != [0; RING_PIXELS * 3] {
                return false;
            }
            self.before = current;
            self.last = current;
        }
        if self.last != *frame {
            if write_ring(&mut self.file, frame).is_err() {
                return false;
            }
            self.last = *frame;
        }
        true
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        if self.last != self.before
            && read_ring(&mut self.file).is_ok_and(|current| current == self.last)
        {
            let _ = write_ring(&mut self.file, &self.before);
        }
    }
}

fn read_ring(file: &mut File) -> io::Result<RingFrame> {
    let mut frame = [0; RING_PIXELS * 3];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut frame)?;
    Ok(frame)
}

fn write_ring(file: &mut File, frame: &RingFrame) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.write_all(frame)
}

struct Segment {
    path: PathBuf,
    max: u8,
}

struct Clock<'a> {
    segments: &'a [Segment; SEGMENTS],
    native: [u8; SEGMENTS],
    last: [u8; SEGMENTS],
    intensity: u8,
    next_readback: Instant,
    redraw: Option<ClockRedraw>,
}

struct ClockRedraw {
    frame: [u8; SEGMENTS],
    since: Instant,
}

impl<'a> Clock<'a> {
    fn open(segments: &'a [Segment]) -> Option<Self> {
        let segments: &[_; SEGMENTS] = segments.try_into().ok()?;
        let native = read_clock(segments)?;
        let brightest = native.iter().copied().max().unwrap_or_default();
        let intensity = if brightest == 0 {
            48
        } else {
            brightest.min(110)
        };
        Some(Self {
            segments,
            native,
            last: native,
            intensity,
            next_readback: Instant::now() + CLOCK_READBACK,
            redraw: None,
        })
    }

    fn show(&mut self, frame: &[u8; SEGMENTS], now: Instant) -> bool {
        if self.redraw.is_some() || now >= self.next_readback {
            self.next_readback = now + CLOCK_READBACK;
            let Some(current) = read_clock(self.segments) else {
                return false;
            };
            if current != self.last {
                // Firmware redraws the clock segment by segment. Leave it alone until the frame settles.
                if !self.redraw.as_ref().is_some_and(|redraw| {
                    redraw.frame == current && now.duration_since(redraw.since) >= CLOCK_SETTLE
                }) {
                    if self
                        .redraw
                        .as_ref()
                        .is_none_or(|redraw| redraw.frame != current)
                    {
                        self.redraw = Some(ClockRedraw {
                            frame: current,
                            since: now,
                        });
                    }
                    return true;
                }
                self.native = current;
                self.last = current;
            }
            self.redraw = None;
        }

        let desired: [u8; SEGMENTS] = std::array::from_fn(|index| {
            let limit = self.intensity.min(self.segments[index].max);
            (u16::from(frame[index]) * u16::from(limit) / 255) as u8
        });
        for (segment, (&last, &value)) in self.segments.iter().zip(self.last.iter().zip(&desired)) {
            if last != value && brightness(&segment.path) != Some(last) {
                self.next_readback = now;
                return true;
            }
        }
        for (segment, (last, value)) in self.segments.iter().zip(self.last.iter_mut().zip(desired))
        {
            if *last != value && set_brightness(&segment.path, value).is_err() {
                return false;
            }
            *last = value;
        }
        true
    }
}

impl Drop for Clock<'_> {
    fn drop(&mut self) {
        // A newer firmware frame wins if it arrived just before the turn ended.
        if read_clock(self.segments) != Some(self.last) {
            return;
        }
        for ((segment, &native), &last) in self.segments.iter().zip(&self.native).zip(&self.last) {
            if native != last && brightness(&segment.path) == Some(last) {
                let _ = set_brightness(&segment.path, native);
            }
        }
    }
}

fn read_clock(segments: &[Segment; SEGMENTS]) -> Option<[u8; SEGMENTS]> {
    let mut frame = [0; SEGMENTS];
    for (value, segment) in frame.iter_mut().zip(segments) {
        *value = brightness(&segment.path)?;
    }
    Some(frame)
}

fn brightness(path: &Path) -> Option<u8> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn set_brightness(path: &Path, value: u8) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
    file.write_all(format!("{value}\n").as_bytes())
}

#[cfg(test)]
mod tests;
