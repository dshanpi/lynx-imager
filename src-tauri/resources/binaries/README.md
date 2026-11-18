Vendor flasher sidecars are copied here by the release workflow.

Expected file names:

- Windows x64: `openixcli.exe`, `openrockcli.exe`, `uuu.exe`, `dfu-util.exe`, and `libusb-1.0.dll`
- Linux x64: `openixcli`, `openrockcli`, `uuu`, `dfu-util`, and `libusb-1.0.so.0`
- macOS ARM64: `openixcli`, `openrockcli`, `uuu`, `dfu-util`, and `libusb-1.0.dylib`

The NXP UUU binary comes from the pinned official `nxp-imx/mfgtools` release.
The workflow verifies a platform-specific SHA-256 digest before packaging it.

STM32 USB DFU support uses dfu-util 0.11. Windows and Linux use the verified
upstream binary archive; macOS ARM64 uses the Homebrew build. Its GPLv2 license
is bundled as `dfu-util-COPYING`.

The Windows release workflow also copies `libusb-1.0.dll` into
`resources/windows/`. Tauri installs that copy beside `lynx-imager.exe`, while the
copy in this directory is installed beside the OpenixCLI/OpenRockCLI sidecars.
The official Windows UUU release is statically linked and does not require that
DLL.

The desktop application also accepts an explicit development-only path in the
device profile. Release packages use the bundled binary by default.

The 2026-10-07 local Windows Rockusb candidate uses the native OpenRockCLI
build from the sibling `openrockcli` workspace. It imports Windows system DLLs
only and opens the manufacturer's `rockusb.sys`; the libusb DLLs remain for
LYNX's other device backends. The candidate's delivery manifest records the
exact sidecar hash. The CI pin above remains the older public revision and
must not be mistaken for this locally built candidate.

The 2026-10-08 fix1 candidate corrects native bulk access to separate PIPE00
(read) and PIPE01 (write) handles. See REQ-REL-041 acceptance and delivery
manifest for regression evidence and the exact binary hash. Windows hardware
acceptance is still pending after the first candidate failed with Win32 6.

The consolidated 2026-10-08 audit candidate supersedes fix1 and includes the
READ_FLASH_INFO response-length, pre-erase validation, strict GPT/IDB readback,
Loader bounds, real poweroff command, and safe extraction corrections. Its
LYNX main executable also fixes late stdout/stderr draining. See REQ-REL-042;
the delivery manifest records the exact hashes, and HIL remains NOT VERIFIED.

The final local 2026-10-08 verification candidate includes REQ-REL-043/044:
Maskrom wait correction and pre-erase Loader verification negotiation. The user
confirmed a successful A1/CM5 flash; a read-only MCP task confirms exit 0 and
successful device-checksum verification. See
`docs/acceptance/WINDOWS_ROCKUSB_HANDOFF.md` and its pinned sidecar digest/local
source revision. This limited board result does not qualify every driver or
installer/update path. REQ-REL-045 keeps commits and artifacts local at the
user's request; remote release pins have not been changed.

The current 0.1.0 reset integration pins OpenRockCLI to
`c081a0987c0b4dde02fe104894c7c3d8989b9368` in `.github/workflows/build.yml`.
The older candidate notes above are historical evidence, not the current CI pin.
Local macOS builds must use the same commit and preserve its license; the
2026-10-14 main integration records build and offline test provenance separately
from hardware acceptance.

REQ-REL-060 / 2026-10-29: V0.1.1 builds OpenixCLI from
`100askTeam/OpenixCLI` commit `4f869200b72d8fecbf6d420326495ff6baa42810`
with its committed Cargo.lock and libefex revision. The legacy v0.1.9 patch
is retained only as historical source, not applied by the current workflow.
The Windows binary tested in preserve7 has SHA-256
`790347ec815bdd691f4b061d327a05277e671355f21ad6b5366ff2469277b284`.
Local release packaging must compare the actual extracted NSIS sidecar to the
reviewed input, not rely on an old cached resources/binaries/openixcli.exe.
This pin does not qualify untested platforms or fresh-state restoration HIL.
