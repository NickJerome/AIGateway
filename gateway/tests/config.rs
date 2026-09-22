use gateway::config::{Config, effective_cpu_quota, threads_from_cpu_max};
#[test]
fn parses_minimal_config() {
    let cfg =
        Config::from_toml("listen = \"0.0.0.0:8080\"\ndata_dir = \"/var/lib/gateway\"\n").unwrap();
    assert_eq!(cfg.listen, "0.0.0.0:8080");
    assert_eq!(cfg.data_dir.to_str().unwrap(), "/var/lib/gateway");
    assert!(cfg.threads >= 1);
}
#[test]
fn explicit_threads_override_wins() {
    let cfg = Config::from_toml("listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nthreads = 3\n")
        .unwrap();
    assert_eq!(cfg.threads, 3);
}
#[test]
fn threads_follow_cpu_quota() {
    assert_eq!(threads_from_cpu_max(Some((200_000, 100_000)), 8), 2);
    assert_eq!(threads_from_cpu_max(None, 8), 8);
    assert_eq!(threads_from_cpu_max(Some((50_000, 100_000)), 8), 1);
    assert_eq!(threads_from_cpu_max(Some((100_000, 100_000)), 8), 1);
}

#[test]
fn limit_overrides_land_in_limits() {
    let cfg = Config::from_toml(
        "listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nthreads = 1\nmemory_limit_mb = 16\nmax_in_flight = 4\n",
    )
    .unwrap();
    assert_eq!(cfg.limits.memory_bytes, 16 * 1024 * 1024);
    assert_eq!(cfg.limits.max_in_flight, 4);
}

#[test]
fn limits_fall_back_to_defaults() {
    let cfg = Config::from_toml("listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nthreads = 1\n")
        .unwrap();
    assert_eq!(cfg.limits.memory_bytes, 32 * 1024 * 1024);
    assert_eq!(cfg.limits.max_in_flight, 32);
}

#[test]
fn unknown_field_is_rejected() {
    let err =
        Config::from_toml("listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nmax_in_fligth = 4\n")
            .unwrap_err();
    assert!(
        format!("{err:#}").contains("max_in_fligth"),
        "error must name the unknown key: {err:#}"
    );
}

#[test]
fn zero_limits_are_rejected() {
    assert!(
        Config::from_toml("listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nmemory_limit_mb = 0\n")
            .is_err()
    );
    assert!(
        Config::from_toml("listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nmax_in_flight = 0\n")
            .is_err()
    );
}

#[test]
fn explicit_zero_threads_is_rejected() {
    assert!(
        Config::from_toml("listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nthreads = 0\n")
            .is_err()
    );
}

#[test]
fn oversized_memory_limit_is_a_config_error_not_a_panic() {
    let result = std::panic::catch_unwind(|| {
        Config::from_toml(
            "listen = \"127.0.0.1:1\"\ndata_dir = \"/tmp/x\"\nmemory_limit_mb = 17592186044416\n",
        )
    });
    let err = result
        .expect("overflow must be a configuration error, not a panic")
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("memory_limit_mb"),
        "error must name the field: {err:#}"
    );
}

/// Builds a fake cgroup v2 tree: each entry is a directory below the root,
/// optionally with a `cpu.max` file. Returns the tempdir as the cgroup root.
fn cgroup_fixture(entries: &[(&str, Option<&str>)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for (sub, cpu_max) in entries {
        let dir = root.path().join(sub);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(text) = cpu_max {
            std::fs::write(dir.join("cpu.max"), text).unwrap();
        }
    }
    root
}

#[test]
fn quota_at_the_cgroup_root_applies() {
    let root = cgroup_fixture(&[("", Some("200000 100000\n"))]);
    assert_eq!(
        effective_cpu_quota(root.path(), "0::/\n"),
        Some((200_000, 100_000))
    );
}

#[test]
fn nested_quota_is_discovered() {
    let root = cgroup_fixture(&[("", Some("max 100000\n")), ("a/b", Some("100000 100000\n"))]);
    assert_eq!(
        effective_cpu_quota(root.path(), "0::/a/b\n"),
        Some((100_000, 100_000))
    );
}

#[test]
fn tighter_ancestor_quota_wins() {
    let root = cgroup_fixture(&[
        ("a", Some("100000 100000\n")),
        ("a/b", Some("400000 100000\n")),
    ]);
    assert_eq!(
        effective_cpu_quota(root.path(), "0::/a/b\n"),
        Some((100_000, 100_000))
    );
}

#[test]
fn all_max_means_no_quota() {
    let root = cgroup_fixture(&[("", Some("max 100000\n")), ("a", Some("max 100000\n"))]);
    assert_eq!(effective_cpu_quota(root.path(), "0::/a\n"), None);
}

#[test]
fn missing_cpu_max_means_no_quota() {
    let root = cgroup_fixture(&[("a/b", None)]);
    assert_eq!(effective_cpu_quota(root.path(), "0::/a/b\n"), None);
}
