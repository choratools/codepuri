# Wave8 — initialized copy regression repaired

Immutable denominator205,497. Strict bodies140,632 +enum300 =140,932(68.58105%).
Original baseline identity gains51,638/losses0; wave7 gains113/losses0.
Full30 report true,34,579classes visited,zero output errors/skips,146.049sec.
All29,672 unchanged sources parse. Uncapped exact-source javac fails62,851errors;
no sources edited/excluded, no fabricated dependencies counted.

Head correctness fixes reserve allocation locals outside branches but reject
reads before exact paired initialization. Copies after successful initialization
now release their own pending markers. Focused premature-read/copy/repeated-init
regression,61 backend library tests,strict workspaceClippy and release build pass.

Goal90 requires44,016 more exact accepted identities. Whole APK compilation and
global initialized-class proof remain incomplete. See JSON ledger/validation.
