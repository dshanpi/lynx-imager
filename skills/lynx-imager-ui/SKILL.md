---
name: lynx-imager-ui
description: Apply the LYNX Imager desktop UI system when designing or changing its React/Tauri interface, including layout, controls, workflow states, and accessibility.
---

# LYNX Imager UI

Use this skill for visible UI work in `src/` and for UI review of the desktop application. It governs the user-facing shell and interaction model. It does not authorize changes to Rust flashing protocols, OpenixCLI, vendor backends, or MCP contracts.

## Product boundary

- The product has three top-level workspaces: `基础烧录`, `高级烧录`, and `连接 AI`.
- The default workspace is the four-step basic flashing flow: `选择板子`, `选择系统`, `配置接口`, and `烧写系统`.
- Do not add the Learning Center, course navigation, account-first onboarding, or the former global tool sidebar to the product shell.
- Keep advanced flashing and AI controls discoverable from the top workspace tabs, while keeping them out of the basic flow.

## Layout rules

- Use the application shell in `src/Components/Imager/ImagerShell.css` and the basic-flow structure in `ImagerPage.tsx`.
- Keep the header, workspace tabs, stepper, content area, action bar, and status footer in a stable hierarchy.
- The basic stepper is a local workflow rail. It must not become a second global navigation system.
- Board selection uses search, vendor filters, a selectable list, and a contextual detail panel. Do not replace it with a dense grid of equal cards unless the product decision explicitly changes.
- Keep the primary action in the bottom action bar. Use one primary action per action group.
- Use existing design tokens and the UI scale gate. Avoid new arbitrary font sizes, weights, colors, shadows, or radii.

## Control and state rules

- Primary actions advance the workflow or start an explicitly confirmed flash task.
- Secondary actions navigate back, inspect details, or open advanced tools.
- Destructive flashing actions require a visible confirmation statement that names the target, image, and storage.
- Every async operation exposes idle, loading, success, failure, and blocked states. Never imply success from a request being accepted.
- Device scanning remains read-only until the user starts the confirmed flash action.
- Error feedback uses an alert region. Progress and task-state changes use a polite status region.
- Native `button`, `input`, and `label` elements must remain keyboard reachable with visible focus states and accessible names.

## Backend boundary

- Reuse the typed IPC commands already used by the basic flow.
- Preserve the existing vendor routes and their saved recipes. UI changes must not silently rewrite loader, storage, erase, verify, reboot, or MCP behavior.
- Keep Allwinner, Rockchip, NXP, ST, and Canaan entry points explicit in the UI when their backend route is available.

## Review checklist

Before committing UI changes, run `npm run check:lynx-imager-ui`, `npm run check:ui-scale`, `npm run typecheck`, and `npm run test:imager`. Review the resulting screen at the Ubuntu preview URL when a layout change is involved.

Read [design-contract.md](references/design-contract.md) when adding a new control, workspace state, or page-level layout.
