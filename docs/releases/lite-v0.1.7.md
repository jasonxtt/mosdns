# lite-v0.1.7

## Summary

`lite-v0.1.7` fixes the maintained Vue UI upstream editor on short screens and zoomed browser layouts.

## Fixed

- kept the upstream editor within the visible viewport
- kept the title and close button visible while long forms scroll
- limited scrolling to the form body so the scrollbar starts below the header instead of overlapping the top-right corner
- mounted the editor at the document body level so parent panel effects cannot change its viewport positioning

## Tests

- previewed the Vue UI through Vite with API requests proxied to the live lite service on `192.168.2.2`
- verified the long `foreign` editor at a 1920x839 browser viewport
- confirmed the modal overlay and card do not scroll while the form body scrolls independently
- rebuilt the maintained Vue UI bundle with `npm run build`

## Upgrade Notes

- this release does **not** require a config change
- existing deployments can update only the binary
