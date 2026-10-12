//! Compile the actual observation hook with an independent register stub.
//! This checks the observer, not (and does not modify) the oracle CPU.
use std::{fs, path::PathBuf, process::Command};

#[test]
fn pc_count_and_combined_gates_preserve_records_and_limits() {
    let root = tempfile::tempdir().unwrap();
    let hook = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/oracle_trace.inc")
        .canonicalize()
        .unwrap();
    let source = root.path().join("stub.c");
    let executable = root.path().join("stub");
    fs::write(&source,format!(r#"
typedef int m68k_register_t;
#define M68K_REG_PC 100
#define M68K_REG_SR 101
#define M68K_REG_D0 0
static unsigned int pc, ordinal;
static unsigned int m68k_get_reg(int r) {{ return r==100 ? pc : r==101 ? 0x2700 : 0x12340000+ordinal*32+r; }}
#include "{}"
int main(void) {{ unsigned int i; for(i=0;i<32;i++) {{ ordinal=i;pc=0x100+(i%10)*2;mdx_trace_instruction(); }} return 0; }}
"#,hook.display())).unwrap();
    assert!(Command::new("cc")
        .args(["-std=c99", "-O2", "-Werror=uninitialized"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    for (index, start_count, start_pc, first, expected) in [
        (0, None, None, 0, vec![0x100u32, 0x102]),
        (1, Some("20"), None, 20, vec![0x100, 0x102]),
        (2, None, Some("0x10a"), 5, vec![0x10a, 0x10c]),
        (3, Some("8"), Some("0x10a"), 15, vec![0x10a, 0x10c]),
    ] {
        let output = root.path().join(format!("{index}.bin"));
        let mut command = Command::new(&executable);
        command
            .env("MDX_ORACLE_TRACE", &output)
            .env("MDX_ORACLE_TRACE_LIMIT", "2")
            .env_remove("MDX_ORACLE_TRACE_START_PC")
            .env_remove("MDX_ORACLE_TRACE_START_COUNT");
        if let Some(value) = start_count {
            command.env("MDX_ORACLE_TRACE_START_COUNT", value);
        }
        if let Some(value) = start_pc {
            command.env("MDX_ORACLE_TRACE_START_PC", value);
        }
        assert!(command.status().unwrap().success());
        let bytes = fs::read(output).unwrap();
        assert_eq!(bytes.len(), 144);
        for (index, (record, pc)) in bytes.chunks_exact(72).zip(expected).enumerate() {
            let word = |index: usize| {
                u32::from_le_bytes(record[index * 4..index * 4 + 4].try_into().unwrap())
            };
            assert_eq!(word(0), pc);
            assert_eq!(word(1), 0x2700);
            assert_eq!(word(2), 0x12340000 + (first + index as u32) * 32);
            assert_eq!(word(17), 0x1234000f + (first + index as u32) * 32);
        }
    }
}
