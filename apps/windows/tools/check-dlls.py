#!/usr/bin/env python3
"""Checks that every DLL the Windows app imports can load on a clean PC.

Walks the .exe/.dll files under the given directories, reads their PE import
tables and resolves each imported DLL the way Windows does for this app: next
to the module, in the app folder, or in System32. The Visual C++ runtime
(msvcp140, vcruntime140 ...) must be bundled or linked statically: a clean
Windows installation does not have it, and this machine's copy in System32
does not count.

    python apps/windows/tools/check-dlls.py apps/windows/dist/SJTUCanvasDownloader

Exit code 1 lists the modules whose imports would fail to load.
"""

from __future__ import annotations

import os
import struct
import sys
from pathlib import Path

VC_RUNTIME = (
    "vcruntime140", "vcruntime140_1", "vcruntime140_threads", "msvcp140", "msvcp140_1", "msvcp140_2",
    "msvcp140_atomic_wait", "msvcp140_codecvt_ids", "concrt140", "vcomp140", "vccorlib140",
)
SYSTEM32 = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32"


def pe_imports(path: Path) -> list[str]:
    """The DLLs a PE file imports (delay-loaded imports are not required)."""
    data = path.read_bytes()
    if data[:2] != b"MZ":
        return []
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        return []
    sections, optional_size = struct.unpack_from("<H12xH", data, pe + 6)
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    directories = optional + (112 if magic == 0x20B else 96)
    table = optional + optional_size

    def offset(rva: int) -> int | None:
        for index in range(sections):
            base = table + index * 40
            virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from("<IIII", data, base + 8)
            if virtual_address <= rva < virtual_address + max(virtual_size, raw_size):
                return rva - virtual_address + raw_pointer
        return None

    rva, size = struct.unpack_from("<II", data, directories + 8)
    start = offset(rva) if rva else None
    names = []
    while start is not None and size:
        name_rva = struct.unpack_from("<I", data, start + 12)[0]
        if name_rva == 0:
            break
        name_start = offset(name_rva)
        if name_start is not None:
            names.append(data[name_start:data.index(b"\0", name_start)].decode("ascii", "replace"))
        start += 20
    return names


def main() -> int:
    targets = [Path(argument).resolve() for argument in sys.argv[1:]]
    if not targets:
        print(__doc__)
        return 2
    modules: list[tuple[Path, Path]] = []
    for target in targets:
        if target.is_file():
            modules.append((target, target.parent))
        else:
            modules += [(path, target) for path in target.rglob("*")
                        if path.suffix.lower() in {".dll", ".exe"} and path.is_file()]

    failures = {}
    for module, root in modules:
        places = [module.parent, root]
        missing = []
        for name in pe_imports(module):
            lower = name.lower()
            if lower.startswith(("api-ms-", "ext-ms-")):
                continue
            if any((place / name).exists() for place in places):
                continue
            if Path(lower).stem in VC_RUNTIME:
                missing.append(f"{name} (Visual C++ runtime, not bundled)")
            elif not (SYSTEM32 / name).exists():
                missing.append(name)
        if missing:
            failures[module] = missing

    print(f"checked {len(modules)} modules")
    for module, missing in sorted(failures.items()):
        print(f"  {module}: {', '.join(missing)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
