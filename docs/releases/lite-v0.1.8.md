# lite-v0.1.8

## Summary

`lite-v0.1.8` fixes the rule-management layout at viewport widths where the
rightmost save action could previously fall outside the visible page.

## Fixed

- allowed the main rule editor to shrink correctly inside the navigation grid
- removed the need to drag the page's horizontal scrollbar to reach the
  rightmost save action at affected desktop resolutions
- kept the "保存全部改动" action on one line

## Tests

- validated the maintained Vue UI at narrow and affected desktop viewport
  widths
- rebuilt the maintained Vue UI bundle with `npm run build`
- ran the full Go test suite with `go test ./...`

## Upgrade Notes

- this release does **not** require a config change
- existing deployments can update only the binary
