fn main() {
    // Ensure exactly one machine feature is enabled
    let machines = [
        (cfg!(feature = "machine-kaonic1s-us"), "kaonic1s-us"),
        (cfg!(feature = "machine-kaonic1s-ks"), "kaonic1s-ks"),
        (cfg!(feature = "machine-host"), "host"),
    ];

    let enabled: Vec<&str> = machines
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| *name)
        .collect();

    match enabled.as_slice() {
        [machine] => {
            // Set cfg flag for easier conditional compilation
            println!("cargo:rustc-cfg=machine=\"{machine}\"");
        }
        [] => panic!(
            "Must enable one of the 'machine-kaonic1s-us', 'machine-kaonic1s-ks' or 'machine-host' features"
        ),
        _ => panic!(
            "Cannot enable more than one machine feature ({}). Use --no-default-features --features machine-<type>",
            enabled.join(", ")
        ),
    }
}
