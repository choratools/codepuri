# Wave10 bounded speed observation

One-class real AppCompat export completed with exact SDK metadata. This is a loaded-host observation, not a controlled speedup benchmark.

The class filter still performed 194286 initial SSA computations across the entire original APK. It emitted one source unit; the class-worker active time was only about 0.14 seconds, while the export took about 71 seconds. Therefore selected-class retry is dominated by global preparation rather than Java rendering.

```text
apk=tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk workers=30 source_release=25 no_io=false unprofiled=false parse_s=0.270271 export_wall_s=70.958677 sources=1 bytes=38838 ordered_checksum=7abf0c8a4e1b2d7c
ssa_cache enabled=true hits=41 misses=194286 entries=194286 failures=0
profile_wall_s=70.854475 delivered_class_profiles=34579 completed_tasks=34579 task_elapsed_sum_s=0.129735 scheduler_wall_sum_s=0.713660 capacity_slot_s=21.409795 worker_lifetime_s=21.248275 active_task_s=0.137308 queue_wait_s=19.361886 dispatch_idle_s=1.749082 effective_active_workers=0.192 capacity_occupancy_pct=0.641 lifetime_active_pct=0.646
serial	alias_build	7.566006	wall_pct=10.663
serial	alias_inventory	1.126164	wall_pct=1.587
serial	cache_release	2.253551	wall_pct=3.176
serial	resolver_prepare	0.620545	wall_pct=0.875
serial	sink_io	0.000290	wall_pct=0.000
serial	source_throws	51.106091	wall_pct=72.022
```

Research candidate: reuse immutable global source-throws/input preparation within an unchanged live session, or establish a dependency-complete lazy source-throws subgraph. The latter must preserve checked contracts, original override obligations, recursive SCC fixed points, and complete-input field uncertainty; skipping unselected methods by name alone is not a valid optimization. No such lazy optimization is implemented or measured here. Existing JavaSession source replay already bypasses completed rendering in the same session.

Evidence: `/tmp/opencode/wave10-appcompat-1791002711028375236`. Source unit retains original #8264 as complete and dependency-complete with exact `super.getText(): Editable`; whole-class compiler completeness is not claimed.
