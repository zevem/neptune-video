---
status: accepted
---

# Native AI produces reviewed edit proposals

Native providers receive bounded project metadata and return typed proposals,
with execution tools disabled, rather than mutating project files or the live
editor. Validate proposals against the captured full project, review their
before/after states, and apply only through one validated reducer batch guarded
by project, context, revision and the exact captured document root. Preserve the
validated after-state for application rather than reducing the commands again:
split, link and group commands allocate identities, so a second reduction would
commit different clip/relationship IDs from those reviewed. The core alone
constructs this prepared edit and commits a changed immutable state as one
ordinary history entry. An unchanged result preserves the current root,
revision, dirty state and redo branch. This preserves exact preview identity, undo/redo and stable
targets while keeping validation and provider processes outside interactive
frames. Backend implementation and its acceptance remain recorded in
[the Native AI contract](../../.scratch/native-ai/spec.md).
Focused contexts explicitly declare omitted objects and retain complete
selected link/group/source relationships, so the metadata budget does not
silently change the meaning of an edit.

The initial adapters reuse native CLI authentication and offer metadata-driven
editing, not audiovisual content analysis. See [native AI](../native-ai.md) for
implemented contracts and backend versus native acceptance evidence.
