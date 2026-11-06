# Settings disclosure

The settings dialog is a place to adjust a terminal, with common controls visible and secondary tasks one interaction away. Existing settings, defaults, backend selection, and authorization stay intact.

## Layout and visual system

- Retain the selected interface style and its semantic tokens. The default palette uses chrome `#0a0a0a`, surface `#111111`, secondary surface `#1a1a1a`, text `#ededed`, muted text `#a0a0a0`, and accent `#006efe`; light styles supply their own values.
- Inherit the application's sans-serif family. Use 20px section titles, 14px group labels, and 12–13px supporting text; preserve 16px mobile form inputs to avoid keyboard zoom.
- Use existing 6px/8px radii, 8/12/16/24px spacing, and one-column content. Separate disclosure rows with quiet rules rather than adding a card for every setting.
- Desktop retains a category sidebar. At 760px and below, a native disclosure shows the current category and reveals the category list on request. Selecting a category closes that list and returns focus to its trigger.

```text
Desktop                           Phone
category sidebar | common fields  settings header
                 | task summaries current category [expand]
                 | expanded task  common fields
                                  task summaries
```

## Behavior

- Terminal: keep instance tabs, cursor, and blinking visible. Group session recovery/backend, input/collaboration, visual effects, and history/diagnostics behind descriptive summaries.
- Fonts and themes: keep selection/preview visible; reveal rendering details and custom theme editing on request.
- Remote hosts: show the connection inventory first. Reveal connection details when creating/selecting a host; keep OpenSSH configuration in a separate disclosure. Preserve disclosure state during controller rerenders and address each save action explicitly.
- Mobile: keep the shortcut preset/editor available. Fold touch behavior, phrases, and clock settings; all can be configured from the dedicated Mobile category on either screen size.
- Tools: retain names, descriptions, and enable switches; place capability metadata inside configuration.
- Closing a native disclosure must retain its mounted form and draft. No height animation or new animation dependency is needed. Chevron feedback uses existing 120ms motion tokens and stops for keyboard/reduced-motion interaction.
- Focus navigation excludes content inside closed disclosures. Breakpoint changes migrate navigation focus to the visible trigger or selected tab. Agent update shortcuts open their containing task and focus its controls/status after the settings dialog opens.
- Use the shared `--lzc-safe-area-inset-*` values for the header, category navigation, and content. Long labels must wrap without overflowing their column.

## Verification

Run frontend/release tests, TypeScript, and the production build. Browser coverage exercises disclosure drafts and saved values, focus loops, both breakpoint directions, Agent update shortcuts, SSH raw-save targeting, 320/375px layouts, landscape, large labels, host-provided safe areas, light/dark themes, and reduced motion. Application version changes do not change the target-local agent compatibility version.
