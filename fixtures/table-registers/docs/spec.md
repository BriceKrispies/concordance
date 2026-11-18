# Widget registers

This specification is normative. MUST and SHALL are used as in RFC 2119.

## Ticket register

The Summary column quotes the ticket title.

| Key | Summary | What the text asks | Business need |
|---|---|---|---|
| T-1 | Some widgets are blank | Widgets show a placeholder when the data is null. The client never throws. | Pages must not look broken. |
| T-2 | Show stats | The list must load `placeholder()` output quickly. | Operators must see the state. |

## Roles

| Role | May do | Where |
|---|---|---|
| app | It reads widgets. It cannot connect to the control database. | its own database |

## Schedules and routines

| Job | Schedule | Routine |
|---|---|---|
| sync | EventBridge `rate(1 hour)` | the job must call `store.find_widget_key(text)` |

The importer must read the `RemoteWidgetTable` table.

## Constraints

`loadWidget()` MUST NOT throw.

`renderAll()` MUST draw every widget.
