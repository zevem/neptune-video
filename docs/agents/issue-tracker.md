# Issue tracker: Local Markdown

Issues and specs live as Markdown files in `.scratch/`.

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`.
- Spec: `.scratch/<feature-slug>/spec.md`.
- Implementation tickets: `.scratch/<feature-slug>/issues/<NN>-<slug>.md`,
  numbered from `01`, one file per ticket.
- Triage state: a `Status:` line near the top of each issue.
  Use the role strings in `triage-labels.md`.
- Work progress: a separate `Progress:` line with `open`, `claimed`, or
  `resolved`. Progress does not replace the canonical triage status.
- Comments: append under a `## Comments` heading.

## Publish to the issue tracker

Create the appropriate spec or ticket file, creating directories as needed.

## Fetch a relevant ticket

Read the referenced file. For an issue number, resolve it within the
referenced feature directory; ask for the feature if ambiguous.

## Wayfinding operations

- Map: `.scratch/<effort>/map.md`, with Notes, Decisions-so-far, and Fog.
- Child ticket: `.scratch/<effort>/issues/NN-<slug>.md`, numbered from `01`.
  Put the question in the body and record `Type:` as
  `research`, `prototype`, `grilling`, or `task`.
- Blocking: record dependencies as `Blocked by: NN, NN`.
  A ticket is unblocked when every dependency has `Progress: resolved`.
- Frontier: choose the lowest-numbered open, unblocked, unclaimed ticket.
- Claim: set `Progress: claimed` and save before working.
- Resolve: append the answer under `## Answer`, set `Progress: resolved`,
  and append a summary and ticket link to Decisions-so-far in the map.
