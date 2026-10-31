//! Exercises `third_eye_client::iobt` (the code the app uses) against a real
//! device: lists candidates, connects, and reports what it reads.
//!
//! Run through `scripts/run_bluetooth_probe.sh <name-or-MAC> [seconds]`
//! so macOS grants Bluetooth access to the bundle.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macOS only.");
}

#[cfg(target_os = "macos")]
fn main() {
    // This executable doubles as the Bluetooth helper, like the app.
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some(third_eye_client::iobt::HELPER_FLAG) {
        std::process::exit(third_eye_client::iobt::run_helper(
            &args.next().unwrap_or_default(),
        ));
    }
    probe();
}

#[cfg(target_os = "macos")]
fn probe() {
    use std::io::{BufRead, BufReader, ErrorKind};
    use std::time::{Duration, Instant};

    use third_eye_client::iobt;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let devices = iobt::list_paired_spp_devices();
    for d in &devices {
        println!(
            "candidate: {} {} node={:?}",
            d.address, d.name, d.serial_node
        );
    }
    let Some(query) = args.first() else {
        return;
    };
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let needle = query.to_lowercase();
    let Some(device) = devices
        .iter()
        .find(|d| d.name.to_lowercase().contains(&needle) || d.address.eq_ignore_ascii_case(query))
    else {
        eprintln!("no candidate matches '{query}'");
        return;
    };

    let started = Instant::now();
    let link = match iobt::connect(&device.address, &std::sync::atomic::AtomicBool::new(false)) {
        Ok(link) => link,
        Err(err) => {
            eprintln!("connect failed after {:?}: {err:#}", started.elapsed());
            return;
        }
    };
    println!("connected in {:?}", started.elapsed());

    let mut reader = BufReader::new(link);
    let (mut lines, mut timeouts) = (0u32, 0u32);
    let mut buf = String::new();
    while started.elapsed() < Duration::from_secs(seconds) {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => {
                println!("EOF (link lost) after {:?}", started.elapsed());
                break;
            }
            Ok(_) => {
                lines += 1;
                if lines <= 3 && buf.starts_with('$') {
                    print!("sample: {buf}");
                }
            }
            Err(e) if e.kind() == ErrorKind::TimedOut => timeouts += 1,
            Err(e) => {
                println!("read error: {e}");
                break;
            }
        }
    }
    println!("lines={lines} timeouts={timeouts}");
    drop(reader);
    println!("closed after {:?}", started.elapsed());
}
