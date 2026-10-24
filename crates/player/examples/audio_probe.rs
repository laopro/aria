use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::OutputStreamBuilder;

fn try_open(device: rodio::cpal::Device) -> String {
    let name = device.name().unwrap_or_default();
    let cfg = match device.default_output_config() {
        Ok(cfg) => cfg,
        Err(e) => return format!("{name}: no default config: {e}"),
    };
    match OutputStreamBuilder::default()
        .with_device(device)
        .with_supported_config(&cfg)
        .open_stream()
    {
        Ok(_) => format!("{name}: OPEN OK ({cfg:?})"),
        Err(e) => format!("{name}: OPEN ERR {e:?}"),
    }
}

fn main() {
    let host = rodio::cpal::default_host();
    println!("host: {:?}", host.id());

    if let Some(device) = host.default_output_device() {
        println!("default -> {}", try_open(device));
    } else {
        println!("NO DEFAULT OUTPUT DEVICE");
    }

    if let Ok(devices) = host.output_devices() {
        for device in devices {
            println!("other   -> {}", try_open(device));
        }
    }
}
