# Seventh wave completed

Export 59792, workspace tests 28279, inventory 23709, coverage 3140, and full
validation 12891 all exited. Coverage: 140,819/205,497 (68.5261 percent).
Baseline delta +51,525/-0; wave6 delta +0/-113. All29,671files parse;
full javac62,688errors.515tests pass/0fail/20ignored.

Wave8 fixes initialized object copies retaining a pending marker. It preserves
reserved-slot read rejection before the exact initializer. Focused regression
and 61 backend library tests pass; release build passed. Full30 and strict
workspace clippy running. Goal active; no unmeasured coverage counted.
