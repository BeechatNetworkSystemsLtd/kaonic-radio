fn main() {
    // Ensure only one machine feature is enabled
    let kaonic1s = cfg!(feature = "machine-kaonic1s");
    let host = cfg!(feature = "machine-host");

    if kaonic1s && host {
        panic!("Cannot enable both 'machine-kaonic1s' and 'machine-host' features. Use --no-default-features --features machine-<type>");
    }

    if !kaonic1s && !host {
        panic!("Must enable either 'machine-kaonic1s' or 'machine-host' feature");
    }

    // Set cfg flag for easier conditional compilation
    if kaonic1s {
        println!("cargo:rustc-cfg=machine=\"kaonic1s\"");
    } else if host {
        println!("cargo:rustc-cfg=machine=\"host\"");
    }
}
