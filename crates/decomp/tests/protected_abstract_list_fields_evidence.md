# Protected inherited field bridge proof

`java_external_fields::resolve_field` and both receiver-retention helpers already
prove original caller subtype, receiver static subtype, actual declaring owner,
field flags/staticness and original/alias-domain name selection. The helper does
not require every classpath field to be public.

The production metadata bridge in `java_integration.rs::Resolver::field` rejected
all external fields with `access_flags & 1 == 0`, independently of those genuine
use-site proofs. This prevents protected `java.util.AbstractList.modCount` even
for subclasses accessing `this`. Do not fix by casting to `AbstractList`: Java
protected receiver rules prohibit that receiver type across packages.

`protected_abstract_list_fields` verifies genuine JDK17 metadata: actual owner
`Ljava/util/AbstractList;`, exact flags `0x84` (protected and transient), nonstatic.
JDK generated subclass read/write runs correctly; parent-typed receiver, sibling
receiver, unrelated caller and static opcode reject. Original differently typed
name shadow and frozen alias-domain shadow reject retention. The invalid
parent-cast Java source fails javac with protected access.

The explicit local Samsung read-only probe verifies original DEX hierarchies
for `Lfq/b$a;` and `Leq/i;`, resolves genuine modCount against JDK17 metadata and
proves retained subclass receiver without any metadata fabrication.

Both tests passed with `--include-ignored`; strict scoped Clippy passed. No
external-field helper implementation was changed or weakened. Production
integration and final APK-wide gain measurement belong to parent. The tests do
not claim a compiled Samsung class or device execution.
