use goodwin_error::{Context, Result, ensure};
use wasmparser::{Encoding, Imports, Parser, Payload, TypeRef};

const PAGE_SIZE: u64 = 64 * 1024;
const MAX_FUNCTIONS: u32 = 10_000;
const MAX_TABLES: u32 = 4;
const MAX_TABLE_ELEMENTS: u64 = 4096;
const MAX_SEGMENTS: u32 = 1000;
const MAX_GLOBALS: u32 = 1000;
const MAX_VALUES: usize = 32;

/// Check Goodwin's module limits before WAMR runs its full Wasm validator.
pub(super) fn module(bytes: &[u8], host_services: bool, max_pages: u32) -> Result<u32> {
    let mut memories = 0;
    let mut imports = 0;
    let mut pages = max_pages;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.context("Invalid WebAssembly module")? {
            Payload::Version { num, encoding, .. } => ensure!(
                num == 1 && encoding == Encoding::Module,
                "Expected WebAssembly 1 bytecode"
            ),
            Payload::TypeSection(section) => {
                for function in section.into_iter_err_on_gc_types() {
                    let function = function?;
                    ensure!(
                        function.params().len() <= MAX_VALUES
                            && function.results().len() <= MAX_VALUES,
                        "WASM function signature is too large"
                    );
                }
            }
            Payload::ImportSection(section) => {
                imports = section.count();
                ensure!(imports <= MAX_FUNCTIONS, "Too many WASM imports");
                for import in section {
                    let Imports::Single(_, import) = import? else {
                        goodwin_error::bail!("WASM compact imports are unsupported");
                    };
                    ensure!(
                        import.module == "goodwin"
                            && matches!(import.ty, TypeRef::Func(_))
                            && (import.name == "play_timer_signal"
                                || (host_services
                                    && matches!(import.name, "host_call" | "host_result"))),
                        "Unsupported WASM import {}.{}",
                        import.module,
                        import.name
                    );
                }
            }
            Payload::FunctionSection(section) => ensure!(
                section.count().saturating_add(imports) <= MAX_FUNCTIONS,
                "Too many WASM functions"
            ),
            Payload::TableSection(section) => {
                ensure!(section.count() <= MAX_TABLES, "Too many WASM tables");
                for table in section {
                    let table = table?.ty;
                    ensure!(
                        !table.table64 && !table.shared && table.initial <= MAX_TABLE_ELEMENTS,
                        "WASM table exceeds its size limit"
                    );
                }
            }
            Payload::MemorySection(section) => {
                memories = section.count();
                ensure!(memories == 1, "Expected one WASM memory");
                for memory in section {
                    let memory = memory?;
                    ensure!(
                        !memory.memory64 && !memory.shared && memory.page_size_log2.is_none(),
                        "Unsupported WASM memory type"
                    );
                    ensure!(
                        memory.initial <= u64::from(max_pages),
                        "WASM memory exceeds its {}-byte limit",
                        u64::from(max_pages) * PAGE_SIZE
                    );
                    if let Some(maximum) = memory.maximum {
                        pages = pages.min(u32::try_from(maximum)?);
                    }
                }
            }
            Payload::GlobalSection(section) => {
                ensure!(section.count() <= MAX_GLOBALS, "Too many WASM globals");
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let name = export?.name;
                    ensure!(
                        !name.contains('\0'),
                        "WASM export name contains a null byte"
                    );
                    ensure!(
                        !matches!(name, "__post_instantiate" | "__wasm_call_ctors"),
                        "WASM plugins cannot export a startup hook"
                    );
                }
            }
            Payload::StartSection { .. } => {
                goodwin_error::bail!("WASM plugins cannot contain a start function");
            }
            Payload::ElementSection(section) => ensure!(
                section.count() <= MAX_SEGMENTS,
                "Too many WASM element segments"
            ),
            Payload::CodeSectionStart { count, size, .. } => ensure!(
                size < 1000 || size >= count.saturating_mul(40),
                "WASM functions are too small on average"
            ),
            Payload::DataSection(section) => {
                ensure!(
                    section.count() <= MAX_SEGMENTS,
                    "Too many WASM data segments"
                );
            }
            _ => {}
        }
    }
    ensure!(memories == 1, "Expected one WASM memory");
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::module;

    #[test]
    fn lower_host_and_guest_memory_limits_win() {
        let bytes = b"\0asm\x01\0\0\0\x05\x04\x01\x01\x02\x08";
        assert_eq!(module(bytes, false, 4).unwrap(), 4);
        assert_eq!(module(bytes, false, 16).unwrap(), 8);
        assert!(module(bytes, false, 1).is_err());
    }

    #[test]
    fn oversized_initial_table_is_rejected_before_loading() {
        let bytes = b"\0asm\x01\0\0\0\x04\x05\x01\x70\x00\x81\x20\x05\x03\x01\x00\x01";
        assert!(
            module(bytes, false, 8)
                .unwrap_err()
                .to_string()
                .contains("table exceeds")
        );
    }
}
