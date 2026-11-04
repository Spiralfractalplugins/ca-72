# The manifest, licences and provenance

*CC0 1.0. Checked against the CA-72 0.3.1's format, November 2026; `docs/skin-packs.md` is the
authority.*

## The manifest, `skin.toml`

The manifest is strict: an unknown field is rejected, because its licence, provenance and
declarations are what a review relies on. (A layout or `[look]` field the plug-in doesn't know
is only noted.) A complete one for a pack an agent made with a user:

```toml
format = 1
id = "org.janedoe.night-panel"
name = "NIGHT PANEL"
version = "1.0.0"
plugin = "ca-72"
summary = "A dark blue panel with copper bands."
license = "CC-BY-4.0"
license_files = ["LICENSE.txt"]

[[creators]]
name = "Jane Doe"
role = "direction"

[[assets]]
path = "layout.toml"
origin = "generated"
tool = "Claude Code (claude-opus-5-5)"
source = "the CA-72 skin pack starter's layout.toml (CC0-1.0)"
note = "arranged by the agent to Jane Doe's brief"

[[assets]]
path = "art/panel.svg"
origin = "generated"
tool = "Claude Code (claude-opus-5-5)"
note = "drawn by the agent to Jane Doe's brief"

[look]
base = "classic"
layout = "layout.toml"
art = "art/panel.svg"
```

## Licence

The user chooses it; you explain the choice if asked. An SPDX expression the checks know
(CC0-1.0, CC-BY-4.0, CC-BY-SA-4.0, the NC and ND variants, GPL, MIT, Apache-2.0, OFL-1.1 and
others listed in `docs/skin-packs.md`), or `LicenseRef-<name>` with the creator's own terms in
the pack. Put the licence's full text in `LICENSE.txt`, named in `license_files`. The
plug-in's GPL does not apply to the pack, and the pack's licence does not apply to the
plug-in. Price and licence are independent: an open pack may be sold.

## Provenance: one `[[assets]]` entry per file

Every file except `skin.toml`, the licence texts, notice files and `.md` files needs one, or
it goes to review (`provenance.missing`).

| Who made it | `origin` | Also give |
|---|---|---|
| You, the agent (wrote it, or rewrote the starter's) | `generated` | `tool`: your product and model, as precisely as you know them ("Codex (gpt-5.2)", "Claude Code (claude-opus-5-5)"); if you don't know the model, say so in the `tool` rather than guess. `source` if it began from the starter. A `note` on what you were asked. |
| An image generator | `generated` | `tool`: the generator and its version |
| The user, by hand | `original` | `author`: the user |
| The user, changing the starter by hand | `derived` | `source = "the CA-72 skin pack starter (CC0-1.0)"`, `license` (the pack's licence: the starter's CC0 lets them choose) |
| Someone else (a font, a photo) | `third-party` | `source` (where from), `license` (its licence), a `[[notices]]` entry if its licence asks |

Work the user and you made together is `generated` if you wrote the file; say in `note`
what the user contributed. Don't add `sha256` unless asked: it rejects the pack the moment the
file changes.

Every generated asset gets a `provenance.generated` review finding. That is correct, not a
mistake to remove: whether AI-made work is protected, and so can be licensed, differs between
countries, and a marketplace's review will ask about it. Never change `origin` to make the
finding go away.

## Id, version and updates

- `id` is the user's own reverse-DNS name (`org.janedoe.night-panel`) and never changes: it is
  how the plug-in, the user's setting and any marketplace know the pack. A new id makes a new
  pack, not an update.
- `com.idlefoundry.*` ids load only with Idle Foundry's first-party signature; a creator never
  uses one. The built-in looks' own ids never load, signed or not.
- `requires = "ca-72 >= x.y.z"` only when a pack truly needs a later CA-72 (an earlier one
  refuses it, `manifest.requires`). Leave it out otherwise.
- An update's layout keeps its `controls` value unless it is remade for a newer revision of
  the CA-72's controls.
- `version` is `major.minor.patch`. Raise it for every release (patch for fixes, minor for
  additions, major for a new look). With the same id installed twice, the plug-in uses the
  newest version, except that a signed pack always beats an unsigned copy, whatever version
  the copy claims.

## Signatures

A publisher adds `signature.toml` (Ed25519, over the pack's id, version, publisher, date and
every file's SHA-256): Idle Foundry for its own packs now, a marketplace later, once there is
one with a review. The plug-in checks it offline. As a creator's agent:

- never write, edit, copy or "fix" a `signature.toml`, and never run `pack_keygen` or
  `pack_sign`: they are the marketplace's tools;
- never change a signed pack: any changed file, id or version makes the plug-in refuse it
  (`signature.invalid`). Work on an unsigned copy;
- remove `signature.toml` only from the user's own copy, at their request, after telling them
  that the copy then loads as unsigned, and that an installed signed copy of the same id will
  still be preferred over it;
- `signature.toml` needs no `[[assets]]` entry.

A signature shows who published the files and that they are the ones reviewed. It is neither
copy protection nor a licence (the pack's licence and the marketplace's terms say what a
buyer may do), and it makes no declaration clearance. The CA-72 has only Idle Foundry's own key
built in, so every pack a creator makes loads as unsigned (`signature.none`, a note), as
manual, unsigned packs are meant to.

## Pack files

A pack file, `<name>.ifskin`, is a ZIP of the pack's folder (its files at the top, or in one
folder at the top), read as the folder is and within the same limits. Write it with
`pack_archive` (SKILL.md, step 6), never by hand with another archiver's options; check it
with `check_pack` as you check the folder. The folder stays the source you edit.

## Declarations

```toml
[declarations]
rights_to_distribute = true
authorizes_customer_licensing = true
agreement = "..."
declared_by = "..."
declared_on = "YYYY-MM-DD"
```

These are the seller's statements, for a marketplace. Leave them out. There is no seller
agreement yet, and only the user can make a declaration, for themselves. Without them the
pack goes to review (`declaration.missing`) but loads and works for its owner. If the user
asks you to add them, explain what they assert (the right to distribute every asset under
the pack's licence, and authority for the marketplace to license it to customers), and that
the checks record them as declared, never as cleared. Write them only with the user's own
name and date, at their explicit request.

## Never in a pack

- A price or store field: `price`, `currency`, `sku`, `license_key`, `drm` (rejected,
  `manifest.store`).
- Trademarks or branding the user has no right to: instrument makers' names, logos, model
  names or lookalike trade dress, in any file or in the name, id or summary.
- Fonts, stock pictures, textures or icons whose licence does not permit this distribution
  (including sale), or whose licence you can't confirm.
- The CA-72's GPL artwork (its pictures, its drawn panel, its official layout), unless the
  pack is under the GPL, which this skill doesn't make.
- A `signature.toml` you wrote, or one carried over into a changed pack.
- Code, archives (a `.ifskin` inside a pack), or any file type but `.toml`, `.txt`, `.md`, `.png`, `.svg`, `.otf`, `.ttf`.
- Personal data beyond the credit the user asks for.
