# VelocityRL Studio UI

The desktop interface follows the supplied NexTest reference: near-black surfaces, neutral typography, rounded panels, and a restrained silver accent. UI UX Pro Max informed the visual hierarchy, focus handling, and motion timing; the requested expressive motion overrides its restrained-motion recommendations.

`ui/studio.css` contains the presentation tokens and responsive styles. `ui/studio.js` handles the startup sequence, live contour background, spatial navigation, selection springs, keyboard navigation, and dialog focus containment. Native commands and game operations stay in the existing application code.

Motion is always enabled as requested, with no toggle or persisted preference. The opaque startup lockup draws three chevrons, charges a line, and retracts as the shell unfolds. Navigation moves old and new screens together using View Transitions, with a directional transform fallback for older WebViews. The sidebar and segmented controls have shared moving selection indicators. Rapid navigation skips an unfinished transition. Catalog cards settle with a small perspective spring and follow pointer position. A bounded canvas draws flowing contours and streaks at approximately 30 fps; it pauses while the document is hidden. Surfaces are solid graphite, with no backdrop blur. No animation dependencies or remote fonts were added.

## Local preview

From the repository root, run `npm run preview` (Python 3 required), then open http://127.0.0.1:4174/ui/?preview . This explicit browser-only mode reads the checked-in catalog and uses the same rendering, search, filter, selection, and pagination functions as the desktop app. Thumbnails load from the existing product image service. Game actions show a read-only notice; they do not invoke native commands. Native Tauri sessions ignore preview mode.

Run `npm run check:ui` for JavaScript syntax validation. Start the native app using the existing `npm run dev` workflow after installing its Node/Rust prerequisites.

## Verification

The initial browser checks covered catalog filtering, search, keyboard selection, navigation, settings dialogs, and layouts at 375 px and the native 1100 px window width. The revised motion pass additionally checks startup cleanup, navigation and subtab changes, always-on motion, and solid dialog surfaces. Native Rocket League integration and a Windows executable build were not exercised as part of this UI change.
