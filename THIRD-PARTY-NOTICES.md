# Third-party notices

Neptune Video's original material is [MIT licensed](LICENSE).
Third-party material retains its own licenses and attribution.

## Fonts and runtime dependencies

The bundled Geist fonts retain their [SIL Open Font License notice](assets/fonts/Geist-LICENSE.txt).
Cargo dependencies retain their declared licenses and upstream notices. The
portable packager writes `dependencies.json` and copies available license texts
into `notices/` alongside the project notice. It records missing texts explicitly;
that inventory is not a completed public-distribution notice review.

FFmpeg and ffprobe are external host tools. The portable bundle does not include
their binaries or change their licensing. Codec redistribution must be reviewed
against the actual build and its dependencies before any bundled codec release.

## Development skills

The `.agents/skills/` material was carried from the read-only Neptune Terminal reference.
These development skills are not included in the application binary bundle.
The upstream license notices below are retained verbatim from that reference.

| Material | Upstream source | Local notice |
| --- | --- | --- |
| `better-ui` | [Jakub Krehel's skills](https://github.com/jakubkrehel/skills/blob/267330e1adfc66a718fb65fa6918c1f06d0a689e/LICENSE) | [MIT](licenses/skills/jakubkrehel-skills-MIT.txt) |
| `emil-design-eng` | [Emil Kowalski's skills](https://github.com/emilkowalski/skills/blob/d16ebe60d09a5ba2afcb7054ede9d0a10c9f6128/LICENSE) | [MIT](licenses/skills/emilkowalski-skills-MIT.txt) |
| `rust-best-practices` guide and adaptations | [Apollo GraphQL's skills](https://github.com/apollographql/skills/blob/222dfc07720227bd0f330bd1b45a9741b1c97bee/LICENSE) | [MIT](licenses/skills/apollographql-skills-MIT.txt) |
| Other imported skill directories | [Matt Pocock's skills](https://github.com/mattpocock/skills/blob/d81f3a183412e71a5b1e84ca21bc1a35eea03a60/LICENSE) | [MIT](licenses/skills/mattpocock-skills-MIT.txt) |
| Adapted `pr` visual examples and placement guidance | [HumanLayer's skills](https://github.com/humanlayer/skills/blob/ca7c8088db69e315a8b2deea43820270457f8f3c/LICENSE) | [MIT](licenses/skills/humanlayer-skills-MIT.txt) |
| Rust handbook reference chapters 1–9 | [Apollo GraphQL's handbook](https://github.com/apollographql/rust-best-practices/blob/eb485a5ddb68e0ded3d79549e994f63ec0a6f6c0/LICENSE) | [Apache-2.0](licenses/skills/apollographql-rust-best-practices-Apache-2.0.txt) |

Adapted material keeps its source, modification and copyright notices. The
project's MIT license does not replace these licenses.
