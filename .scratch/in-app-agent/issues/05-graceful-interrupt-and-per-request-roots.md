# Graceful interrupt and roots per request

Status: needs-triage
Progress: open
Type: task
Blocked by: 01, 02

## Scope

Not yet offered:

- Stop that interrupts the turn and keeps the conversation. Today Stop and
  Cancel kill the provider process and end the conversation; the next
  request starts fresh. Claude's stream-json input and Codex's
  app-server would each need their own interrupt, and the provider must
  not keep a tool call in flight against the window after it.
- ~~Roots that follow the project during a conversation.~~ Done by
  [issue 06](06-same-reach-as-an-outside-agent.md): the window writes the
  folders to a roots file that the server reads again whenever it changes,
  so importing from a new folder or adding one in Settings › Agent ›
  Folders keeps the conversation.
- Keeping `INSTRUCTIONS` out of the provider's process arguments, where
  `ps` shows them. They are Neptune Video's own text with no person data.

## Result

None yet.

## Comments

2026-10-14: Opened. Decided on the owner's behalf (2026-10-14): Stop takes
effect at once at the cost of the conversation, and roots stay fixed for a
conversation, until this issue is taken up.

2026-10-15: The roots part is done by
[issue 06](06-same-reach-as-an-outside-agent.md) (live `--roots-file`,
folders the person adds in Settings › Agent › Folders). Graceful interrupt
and keeping `INSTRUCTIONS` out of the process arguments remain open.
