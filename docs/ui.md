# Boot UI

`radio-ui` is a `no_std`, heap-free state machine over logical key edges and
bounded semantic views. Physical adapters will later map evidenced key hardware
to `Menu`, `Up`, `Down`, `Confirm`, and `Back`, and map views to a particular
display. Work Package 5 defines neither mapping.

## Hidden permission entry

The permission editor is selected only when the initial held-key set is exactly
`Menu+Back`. Missing either key or holding any additional logical key selects
the normal runtime path permanently for that boot. Once accepted, the UI shows
`ReleaseBootKeys` until every key is released. Duplicate press/release edges are
ignored, and later runtime key sequences can never enter the editor.

This gesture demonstrates a deliberate boot-time physical-presence workflow
and prevents ordinary navigation from exposing the menu. It is not a password,
authentication mechanism, or physical key-matrix claim.

## Permission editing

The fixed selectable order is licence-free plan, amateur, marine,
aeronautical, business, and experimental. `Never` is not selectable and remains
denied in every `PermissionSet`.

- `Up` and `Down` wrap through the fixed class list.
- `Confirm` toggles only the selected draft class.
- `Back` cancels, restores the loaded draft, and emits no persistence bytes.
- `Menu` deliberately saves the complete draft with the next monotonic
  generation. Generation exhaustion refuses the save and emits no bytes.

Views contain only the selected class, enabled/changed state, bounded save
error, and saved generation. Pixel geometry, strings, fonts, and layout are
adapter concerns.

Invalid permission bytes initialize the editor with deny-all permissions and
generation zero. A successful save emits the existing versioned, inverted,
CRC-protected `StoredPermissions` record. The UI owns no live `TxPolicy`, cannot
construct `TxAuthorisation`, and does not activate saved permissions. A later
boot must validate and load the new record. Serial configuration has no
permission-object mutation path.
