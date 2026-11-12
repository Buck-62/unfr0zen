use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

const FRAME: &str = "/media/data/nest-photo/photo.frame";
const OWNER: &str = "/var/run/unfr0zen/screen.owner";
const GRAPHICS: &str = "/sys/class/graphics/fb0";
const STRIDE: usize = 1920;
const HEIGHT: usize = 480;

struct ScreenOwner;

impl Drop for ScreenOwner {
    fn drop(&mut self) {
        let _ = fs::remove_file(OWNER);
    }
}

fn button() -> io::Result<File> {
    for entry in fs::read_dir("/sys/class/input")? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        let Ok(keys) = fs::read_to_string(entry.path().join("device/capabilities/key")) else {
            continue;
        };
        let key_enter = keys.split_whitespace().last()
            .and_then(|bits| u64::from_str_radix(bits, 16).ok())
            .is_some_and(|bits| bits & (1 << 28) != 0);
        if key_enter {
            let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK)
                .open(Path::new("/dev/input").join(entry.file_name()))?;
            // EVIOCGRAB keeps the exit press from reaching the Settings menu.
            if unsafe { libc::ioctl(file.as_raw_fd(), 0x40044590u32 as _, 1) } != 0 {
                return Err(io::Error::last_os_error());
            }
            return Ok(file);
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "Nest ring button unavailable"))
}

pub fn show() -> io::Result<()> {
    let frame = fs::read(FRAME)?;
    let read_graphics = |name| fs::read_to_string(Path::new(GRAPHICS).join(name));
    if frame.len() != STRIDE * HEIGHT
        || read_graphics("stride")?.trim() != "1920"
        || read_graphics("bits_per_pixel")?.trim() != "32"
        || read_graphics("virtual_size")?.trim() != "480,1280"
    {
        return Err(io::Error::other("photo requires the gen 3 display and a 480×480 framebuffer image"));
    }
    let mut display = OpenOptions::new().write(true).open("/dev/fb0")?;
    let mut button = button()?;
    let mut owner = OpenOptions::new().create_new(true).write(true).open(OWNER)?;
    let _owner = ScreenOwner;
    write!(owner, "{}", std::process::id())?;
    drop(owner);
    std::thread::sleep(Duration::from_secs(1));
    let pan = read_graphics("pan")?;
    let row = pan.trim().strip_prefix("0,").and_then(|row| row.parse::<usize>().ok())
        .filter(|&row| row <= 1280 - HEIGHT)
        .ok_or_else(|| io::Error::other("unexpected display offset"))?;
    let brightness = "/sys/class/backlight/1-0063/brightness";
    if fs::read_to_string(brightness)?.trim() == "0" {
        fs::write(brightness, "1842\n")?;
    }
    display.seek(SeekFrom::Start((row * STRIDE) as u64))?;
    display.write_all(&frame)?;

    let mut pressed = None;
    let mut backlight_check = Instant::now();
    loop {
        let mut event = [0u8; 16];
        match button.read(&mut event) {
            Ok(16) if event[8..12] == [1, 0, 28, 0] => match event[12..16] {
                [1, 0, 0, 0] => pressed = Some(Instant::now()),
                [0, 0, 0, 0] if pressed.is_some_and(|at: Instant| at.elapsed() >= Duration::from_secs(3)) => return Ok(()),
                [0, 0, 0, 0] => pressed = None,
                _ => {}
            },
            Ok(16) => {}
            Ok(_) => return Err(io::Error::other("incomplete ring event")),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        if backlight_check.elapsed() >= Duration::from_millis(500) {
            if fs::read_to_string(brightness)?.trim() == "0" {
                fs::write(brightness, "1842\n")?;
            }
            backlight_check = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
