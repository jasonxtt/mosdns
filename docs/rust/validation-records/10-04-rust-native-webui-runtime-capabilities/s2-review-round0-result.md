# S2 initial exact-source review

Range 9e9254e0..2e1b9387. Reviewer same dedicated chat.

P2-1 [open] — /log/ redirect drops query; real HTTP test only bare /log/. Reviewer calls query preservation a frozen S2 requirement; design explicitly mentions it for external mounts, but preserving safe query is compatible with root redirect workflow and adopted within S2.
P2-2 [open] — static semaphore precedes routing decisions, so saturation rewrites405/301/404 to503. Valid body-budget/routing boundary issue.

FINAL: FAIL
