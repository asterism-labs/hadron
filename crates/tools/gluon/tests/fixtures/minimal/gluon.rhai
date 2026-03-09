// Minimal test project for gluon integration tests.

project("test-project", "0.1.0");

target("x86_64-unknown-test", "targets/x86_64-unknown-test.json");

profile("default")
    .target("x86_64-unknown-test")
    .opt_level(0)
    .debug_info(true);

config_bool("TEST_OPT", true)
    .help("A test option");

config_bool("OTHER_OPT", false)
    .help("Another test option");

let g = group("kernel")
    .target("x86_64-unknown-test")
    .edition("2024");

g.add("test-crate", "crates/test-crate");

pipeline()
    .stage("kernel", ["kernel"]);
