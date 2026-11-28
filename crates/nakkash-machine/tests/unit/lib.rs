use super::parse::{self, Psi};
use super::*;

#[test]
fn meminfo_figures_are_kib_turned_to_bytes() {
    let text = "MemTotal:       65011712 kB\nMemFree:  1024 kB\nMemAvailable:   38797312 kB\n";
    assert_eq!(
        parse::meminfo_bytes(text, "MemTotal"),
        Some(65_011_712 * 1024)
    );
    assert_eq!(
        parse::meminfo_bytes(text, "MemAvailable"),
        Some(38_797_312 * 1024)
    );
    assert_eq!(parse::meminfo_bytes(text, "Mem"), None);
}

#[test]
fn io_and_vmstat_lines_are_read_by_key() {
    let io = "rchar: 10\nwchar: 20\nread_bytes: 4096\nwrite_bytes: 0\ncancelled_write_bytes: 0\n";
    assert_eq!(parse::keyed(io, "write_bytes"), Some(0));
    assert_eq!(parse::keyed(io, "read_bytes"), Some(4096));
    let vmstat = "pswpin 12\npswpout 34\n";
    assert_eq!(parse::keyed(vmstat, "pswpout"), Some(34));
    assert_eq!(parse::keyed(vmstat, "pswp"), None);
}

#[test]
fn a_cgroup_without_a_limit_says_max() {
    assert_eq!(parse::limit("max\n"), None);
    assert_eq!(parse::limit("9307357184\n"), Some(9_307_357_184));
    assert_eq!(parse::cgroup_path("0::/box/job-12\n"), Some("/box/job-12"));
    assert_eq!(parse::cgroup_path("1:memory:/x\n"), None);
}

#[test]
fn stall_information_reads_as_pressure() {
    let calm = "some avg10=0.00 avg60=0.00 avg300=0.00 total=3013461757\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=2716339398\n";
    let psi = Psi::parse(calm).expect("parsed");
    assert_eq!(psi.pressure(), Pressure::Normal);
    let some = Psi::parse(
        "some avg10=12.5 avg60=1 avg300=0 total=1\nfull avg10=1.0 avg60=0 avg300=0 total=1\n",
    );
    assert_eq!(some.map(Psi::pressure), Some(Pressure::Warning));
    let full = Psi::parse(
        "some avg10=40 avg60=1 avg300=0 total=1\nfull avg10=7.5 avg60=0 avg300=0 total=1\n",
    );
    assert_eq!(full.map(Psi::pressure), Some(Pressure::Critical));
    assert_eq!(Psi::parse("nothing"), None);
}

#[test]
fn shares_of_memory_read_as_pressure() {
    assert_eq!(parse::used_pressure(50, 100), Pressure::Normal);
    assert_eq!(parse::used_pressure(86, 100), Pressure::Warning);
    assert_eq!(parse::used_pressure(96, 100), Pressure::Critical);
    assert_eq!(parse::available_pressure(50, 100), Pressure::Normal);
    assert_eq!(parse::available_pressure(8, 100), Pressure::Warning);
    assert_eq!(parse::available_pressure(4, 100), Pressure::Critical);
    assert_eq!(parse::kernel_level(1), Pressure::Normal);
    assert_eq!(parse::kernel_level(2), Pressure::Warning);
    assert_eq!(parse::kernel_level(4), Pressure::Critical);
}

#[test]
fn the_default_budget_is_a_quarter_of_memory_at_most_four_gib() {
    assert_eq!(budget_for(Some(36 << 30)), 4 << 30);
    assert_eq!(budget_for(Some(16 << 30)), 4 << 30);
    assert_eq!(budget_for(Some(8 << 30)), 2 << 30);
    assert_eq!(budget_for(None), DEFAULT_BUDGET_UNKNOWN);
    assert!(default_budget() > 0);
}

#[test]
fn swap_changes_read_as_a_line() {
    let before = Swap::Pages { ins: 10, outs: 5 };
    let after = Swap::Pages { ins: 12, outs: 5 };
    assert_eq!(after.since(before), "2 pages in, 0 out (machine-wide)");
    assert_eq!(
        Swap::Used(2 * MIB).since(Swap::Used(MIB)),
        "swap used +1.0 MiB (machine-wide, 2.0 MiB now)"
    );
    assert_eq!(Swap::Used(0).since(before), "unknown");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn the_machine_says_what_it_holds_and_what_was_used() {
    let memory = memory().expect("the system says how much memory it has");
    assert!(memory >= 256 * MIB, "{memory}");
    assert!(default_threads() >= 1);
    // Only that it reads; any level is possible on a busy machine.
    let _ = pressure();
    // Touch some memory so the peak is above nothing.
    let held = vec![1u8; 8 << 20];
    let usage = usage();
    assert!(usage.peak_resident >= held.len() as u64, "{usage:?}");
    assert!(usage.written.is_some(), "{usage:?}");
    assert!(swap().is_some());
    let resident = resident().expect("the system says what is resident");
    assert!(
        resident >= held.len() as u64 && resident <= usage.peak_resident.max(resident),
        "{resident}"
    );
}

#[test]
fn an_environment_cap_limits_the_default_threads() {
    // A job runner caps its child's threads in the environment: the
    // default keeps under it, and a cap that does not read changes nothing.
    assert_eq!(capped(12, Some("2")), 2);
    assert_eq!(capped(12, Some(" 5 ")), 5);
    assert_eq!(capped(4, Some("16")), 4);
    assert_eq!(capped(12, Some("0")), 12);
    assert_eq!(capped(12, Some("many")), 12);
    assert_eq!(capped(12, None), 12);
    assert_eq!(capped(0, None), 1);
}

/// The free space of an existing folder's file system is known on every
/// system the project runs on, and none for a path that does not exist or
/// cannot be named to the system.
#[test]
fn free_space_is_known_for_an_existing_folder_only() {
    let here = std::env::temp_dir();
    let free = free_space(&here);
    #[cfg(unix)]
    assert!(free.is_some_and(|bytes| bytes > 0), "{free:?}");
    let _ = free;
    assert_eq!(free_space(&here.join("nakkash-no-such-folder/below")), None);
    assert_eq!(free_space(std::path::Path::new("a\0b")), None);
}

/// A system is named by the versions of its parts, in order; a part the
/// system left blank leaves the whole unknown, never named by the rest.
#[rstest::rstest]
#[case::macos([("macOS", "26.3"), ("build", "25D125")], Some("macOS 26.3, build 25D125"))]
#[case::linux(
    [("glibc", "2.39"), ("Linux", "6.8.0-49-generic")],
    Some("glibc 2.39, Linux 6.8.0-49-generic")
)]
#[case::trimmed([("macOS", " 26.3\n"), ("build", "25D125 ")], Some("macOS 26.3, build 25D125"))]
#[case::blank_version([("macOS", ""), ("build", "25D125")], None)]
#[case::blank_build([("macOS", "26.3"), ("build", "  ")], None)]
#[case::blank_kernel([("glibc", "2.39"), ("Linux", "")], None)]
fn a_system_is_named_by_its_parts_or_unknown(
    #[case] parts: [(&str, &str); 2],
    #[case] named: Option<&str>,
) {
    assert_eq!(parse::system_name(parts).as_deref(), named);
}

/// A Linux system is named by its C library's version, its maths
/// library's content hash and its kernel's release; a blank hash leaves it
/// unknown, as a blank version does.
#[rstest::rstest]
#[case::named(
    "0123456789abcdef0123456789abcdef",
    Some("glibc 2.36, libm 0123456789abcdef0123456789abcdef, Linux 6.1.0-28-amd64")
)]
#[case::blank_hash("", None)]
fn a_linux_system_is_named_with_its_maths_library(#[case] libm: &str, #[case] named: Option<&str>) {
    let parts = [
        ("glibc", "2.36"),
        ("libm", libm),
        ("Linux", "6.1.0-28-amd64"),
    ];
    assert_eq!(parse::system_name(parts).as_deref(), named);
}

/// This machine names its system as its kind of system does, the same at
/// every call; with glibc, by the content of the maths library it loaded.
#[test]
fn this_machine_names_its_system() {
    let system = system();
    #[cfg(target_os = "macos")]
    let prefix = Some("macOS ");
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    let prefix = Some("glibc ");
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
    let prefix: Option<&str> = None;
    match prefix {
        Some(prefix) => {
            let named = system.clone().expect("this system names itself");
            assert!(named.starts_with(prefix), "{named}");
            assert!(named.contains(", "), "{named}");
        }
        None => assert_eq!(system, None),
    }
    assert_eq!(super::system(), system);
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let maths = loader::maths_library().expect("glibc's maths library is loaded");
        let libm = format!(", libm {:032x}, Linux ", maths.hash);
        assert!(
            system.as_deref().is_some_and(|s| s.contains(&libm)),
            "{system:?}"
        );
    }
}
