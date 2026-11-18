# LYNX Imager design contract

## Shared geometry

- Application header: 72px row with brand, three workspace tabs, and compact utility actions.
- Basic workflow rail: approximately 224–240px wide with four numbered steps.
- Main page padding: 24–44px depending on window width.
- Compact control heights: 32–40px. Primary actions may use the existing 40px token.
- Use the repository's accepted typography scale. `npm run check:ui-scale` is authoritative.

## Action hierarchy

| Level | Use | Examples |
| --- | --- | --- |
| Primary | Advance the current workflow or submit a confirmed task | `下一步：选择系统`, `开始烧写并校验` |
| Secondary | Move back or inspect a non-destructive detail | `上一步`, `查看板型说明` |
| Utility | Help, settings, history, filters, and file selection | `?`, `⚙`, `选择文件` |
| Destructive | Erase/write/reboot actions after confirmation | `开始烧写并校验` |

Use text and state together. Color alone must not carry a destructive, selected, or failed meaning.

## Workflow state matrix

| State | Required visible feedback | Allowed next action |
| --- | --- | --- |
| Idle | Current step, selected board, actionable primary button | Continue when prerequisites are complete |
| Incomplete | Missing field or disabled primary button with a nearby reason | Fill the missing prerequisite |
| Loading | Busy label and disabled repeat action | Wait or cancel when the backend supports cancellation |
| Success | Explicit completed phase and task identifier where available | Continue to next phase or inspect result |
| Failure | Alert text naming the failed phase and preserving the device-safe state | Retry only when the backend says it is safe |
| Blocked | Clear safety reason, no automatic retry, no hardware mutation | Resolve the named blocker |

## New UI review questions

1. Can a first-time user identify the next action without opening Advanced or AI?
2. Does each control have one clear purpose and an accessible name?
3. Does a failed or blocked task communicate whether the device was touched?
4. Does the page preserve the existing IPC and vendor route semantics?
5. Does the layout remain usable at the repository's supported narrow width?
