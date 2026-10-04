# Wave9 — measured constructor/super/static/allocation improvements

Denominator205,497 remains immutable. Accepted144,829(70.4774%):144,529
strict complete bodies+300 enum equivalents. Baseline+55,535/-0; wave8+3,897/-0.
All29,714 unchanged sources parse. Fulluncappedjavac fails63,213errors.
Full30:150.872sec,all34,579classesseen,errors/skips0. Workspace521passed,
0failed,23ignored; strictworkspace/alltargetClippy and release build pass.

Production paths: exact numeric allocation args and operand-type pinning;
contextual constructor args; per-path branching static final initialization;
closed reachable constructor subset independent of unrelated failed siblings;
complete intermediate ancestor inventory for exact inherited super targets.
Unknown2135write observations not erased; no complete initialized-class claim.
Actual witnesses: Persister ctor34035 complete; a6.c228/262 complete;
os/i$b$a57209 complete. Static14runtime harness has explicit flagowner boundary.
90% stillrequires40,119 more identities. EntireAPKcompilation/runtime unproven.
