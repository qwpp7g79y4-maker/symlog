//! Eén byte boven de gedocumenteerde invoerlimiet is exit 2.
//! Het bestand is sparse, staat in `/tmp`, en wordt aan het einde verwijderd.
//! Het is geen bestand in de repository.

use std::time::Duration;

use assert_cmd::Command;

#[test]
fn log_over_the_byte_limit_exits_2() {
    const LIMIT: u64 = 268435456;
    assert_eq!(symlog::startup::MAX_INPUT_BYTES, LIMIT);

    struct Gone(std::path::PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let path = Gone(std::path::PathBuf::from(format!(
        "/tmp/symlog-w46-over-limit-{}.log",
        std::process::id()
    )));
    let file = std::fs::File::create(&path.0).expect("create over-limit file in /tmp");
    file.set_len(LIMIT + 1)
        .expect("sparse length one byte over the limit");
    drop(file);

    let shown = path.0.to_str().expect("path is utf-8");
    let assert = Command::cargo_bin("symlog")
        .expect("symlog binary")
        .timeout(Duration::from_secs(5))
        .arg(shown)
        .assert()
        .failure()
        .code(2)
        .stdout("");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains(&format!("boven de limiet van {LIMIT} bytes")),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("is {} bytes", LIMIT + 1)),
        "{stderr}"
    );
    assert!(stderr.contains("niet ingelezen"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}
