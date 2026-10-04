# Interface-super source boundaries (read-only Rust investigation)

Evidence base: frozen wave35 ledger, same Samsung Pass APK, pinned local JADX. No Rust edits, builds, whole export, or aggregate gain claims. The wave36 export was live while this document was prepared.

## Exact accounting and candidate inventory

214 selected-target rejects: **175 class-instance owners, 39 interface-static owners**, determined from genuine original DEX class/method flags.

Artifact: `/tmp/opencode/wave36-interface-super-inventory.json`; collector `/tmp/opencode/wave36-interface-inventory.py`.

Within the 175 instance owners, original code-unit candidate scanning gives:

| Candidate shape | Methods |
|---|---:|
| Single input-interface target, exact direct superinterface | 87 |
| Single non-input-interface/class target | 60 |
| Input-interface target absent from caller direct-interface ancestry | 26 |
| Multiple super call sites | 2 |

These are **candidate counts, not source-selection proofs**: extraction scans opcode units rather than implementing a complete DEX decoder. Every extension must consume the existing genuine decoded instruction identity. The non-input-interface category includes genuine external interfaces and must not be interpreted as 60 confirmed class targets.

Of the 87 input direct-interface candidates, **86 target declarations already have complete_body**; one target is rejected: classes3#21002 SettingsViewModelImpl.startVaultService -> SettingsViewModel#20884. Its dispatch can be proved separately, but body/dependency availability must not be fabricated.

Two external direct-interface candidates in the other category: classes3#12282/#12283, ServiceConnection.onBindingDied/onNullBinding. Android35 declarations must provide authoritative interface/default/access metadata before acceptance.

No multi-level direct-interface ancestry path was found in the 87 single-target input candidates. There are 26 targets reached through superclass inheritance, e.g. classes.dex#32166 FloatingBottomLayout.getBlurTargetView -> j.a.getBlurTargetView; caller has no direct interfaces and extends FloatingGroupLayout. **Do not emit j.a.super** for this caller. A separately proved plain-super selection through the superclass default inheritance might recover it; unknown or ambiguous hierarchy remains closed.

classes2#29441 MultiSelectionListDialog.onCreate has two distinct actual super sites: offset0 Fragment q.onCreate, offset3 directly implemented InitialisableDialog.onCreate. Preserve both calls and their original order.

## Decisive two Samsung originals

- classes2#24454 CertificateRepositoryImpl.getStgEnvironment()Z, flags0x41, code_item2882784.
- classes2#24225 CertificateRepository.access$getStgEnvironment$jd(CertificateRepository)Z, flags0x1009, code_item2902240.
- Both words `[0x106f,24243,0,0x000a,0x000f]`: invoke-super offset0, exact target CertificateRepository#getStgEnvironment()Z, move-result3, return4.
- Implementation class flags0x11, superclassObject, direct interfaceCertificateRepository.
- Interface flags0x601, extendsCertificateValidityChecker; target24243 is concrete public default and already emitted with original MethodKey provenance, using DebugUtils.Companion.getInstance().isDiServerStg().

Pinned output `/tmp/opencode/wave35-super-CertificateRepositoryImpl.java:2231` uses invalid `super.getStgEnvironment()` although class implements the interface. `/tmp/opencode/wave35-super-CertificateRepository.java:24` puts invalid `CertificateRepository.super.getStgEnvironment()` in a static DefaultImpls method. JADX is semantic evidence, not a compilability oracle here.

## Proposed bounded instance implementation and ownership

1. **Worker owns java_super_resolution.rs + dedicated tests:** extend genuine SuperClass/provider evidence with direct interfaces; select exact interface default separately from existing class-only selection. Require complete acyclic hierarchy, direct-superinterface qualifier, maximal-specific/default uniqueness, executable public nonstatic/nonprivate target, exact original parameter/return descriptor and generic/bridge/alias proof. Retain exact target metadata/throws and distinguish qualifier from declaring owner. Source publication completeness remains a separate dependency contract.
2. **Worker owns special_invokes.rs + dedicated backend tests:** represent checked qualified-interface source dispatch without conflating it with ordinary virtual or plain class super.
3. **Parent owns tiny lib.rs emitter/JavaResolver and java_integration.rs wiring:** expose selected source qualifier tied to the exact existing SuperInvocation contract; emit validated alias qualifier + `.super`, retain descriptor argument casts, original incoming receiver identity, selected throws, access proof, invocation position/effects. No symbolic raw declaration fallback.
4. Other parent consumers (constructor suffix/source cleanup/catch contracts) reuse the same exact selected metadata; no independent guessed selection.

Potential API: checked source dispatch enum `ClassSuper` versus `InterfaceSuper { qualifier }`, accompanying selected MethodKey. Source qualifier is **direct implemented interface**, not automatically method declaring owner. Existing superclass behavior defaults unchanged; unavailable new interface proof rejects.

## Required positive and negative checks

Positive fixture: I default m(), C implementsI overridesm() and recovered wrapper callsI.super.m(); runtime subclass override must not redirect that call. Test null/effects/arguments and selected checked throws. Actual24454 source acceptance must preserve boolean default body and original signature; whole original class compilation is separate.

Negatives: interface qualification not direct; redundant I when J extendsI also directly implemented; two conflicting unrelated defaults; abstract redeclaration suppressing inherited default; missing/cyclic ancestry; superclass override changing Java selection; altered return/overload/bridge target; alias collision; static helper receiver; unrelated receiver; target missing from genuine inventory; selected throws absent/disagreeing.

## Deferred static39

Static interface access helper24225 cannot legally invoke qualified-super and cannot be replaced with receiver.getStgEnvironment() (would dispatch to implementation bridge and recurse). Preserve it rejected until an explicit exact default-body extraction/helper transformation proves receiver identity, source signature, initialization, effects, exception contract, and all method dependencies. Do not relax to virtual dispatch or copy invalid JADX syntax.
