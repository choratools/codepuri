# Wave51 compiler triage: source correctness candidate

Input: `/tmp/opencode/samsung-pass-100-wave50-runtime-metadata/validation25/full-javac.log`
Validation: `/tmp/opencode/samsung-pass-100-wave50-runtime-metadata/validation25/validation.json`
Source root: `/tmp/opencode/samsung-pass-100-wave50-runtime-metadata/java25/sources`

## Classification

The full javac run has 44,884 diagnostics. Most are dependency/publication gaps:

- `cannot find symbol`: 40,920
- `package ... does not exist`: many package groups
- Missing return-only/covariant method declarations cause secondary incompatible types, e.g. `Arrays.constantTimeAreEqual([B,[B)` unavailable, so byte[] callers try the emitted long[] overload.

The clearest repeated emitted-source correctness error is Java method aliasing for DEX return-only collisions. There are 49 `cannot implement` diagnostics. The root pattern is that a private/synthetic/narrow bridge method keeps the Java source name while the method that must satisfy an interface/super declaration is renamed with `__2`.

## Exact Samsung app representative

Compiler errors:

```text
SamsungPassServiceImpl.java:156: getRelayPassService() in SamsungPassServiceImpl cannot implement getRelayPassService() in ISamsungPassService
  attempting to assign weaker access privileges; was public
SamsungPassServiceImpl.java:143: getDigitalWalletPassService() ... weaker access privileges
SamsungPassServiceImpl.java:130: getCryptoService() ... weaker access privileges
SamsungPassServiceImpl.java:68: getAuthService() ... weaker access privileges
```

Generated source:

```java
public final class SamsungPassServiceImpl extends ISamsungPassService$Stub { ... }
private final AuthServiceImpl getAuthService() { ... }        // method 13116
public IAuthService getAuthService__2() { ... }               // method 13117
```

Original DEX identities from method ledger:

```text
classes3.dex#13116 Lcom/samsung/android/samsungpass/passservice/service/SamsungPassServiceImpl;->getAuthService()Lcom/samsung/android/samsungpass/passservice/service/AuthServiceImpl; access_flags=18 private final
classes3.dex#13117 Lcom/samsung/android/samsungpass/passservice/service/SamsungPassServiceImpl;->getAuthService()Lcom/samsung/android/samsungpass/sdk/IAuthService; access_flags=1 public
classes3.dex#20227 Lcom/samsung/android/samsungpass/sdk/ISamsungPassService;->getAuthService()Lcom/samsung/android/samsungpass/sdk/IAuthService; public abstract
```

The public method must own the Java source name `getAuthService`; the private delegate can be renamed because it is not a Java override obligation.

JADX 1.5.1 comparison:

- Command: `jadx -q --single-class com.samsung.android.samsungpass.passservice.service.SamsungPassServiceImpl ...`
- Output: `/tmp/opencode/wave51-jadx-samsungpassservice/SamsungPassServiceImpl.java`
- JADX also detects the collision and annotates the public method as renamed:

```java
private final AuthServiceImpl getAuthService() { ... }
/* renamed from: getAuthService, reason: collision with other method in class */
public IAuthService mo189getAuthService() { ... }
```

JADX is not compile-oriented here, but it independently confirms the same DEX return-only name collision.

## Additional class-family representative

`qr/k0$type.java` has the same alias bug for bridge/interface methods:

```text
classes3.dex#58486 Lqr/k0;->c(Lkt/f2;)Lnr/n; access_flags=4161 public bridge synthetic
classes3.dex#58487 Lqr/k0;->c(Lkt/f2;)Lnr/y0; access_flags=1 public
classes3.dex#56606 Lnr/y0;->c(Lkt/f2;)Lnr/y0; public abstract
```

Generated source puts the bridge at `c(...)` and the interface implementation at `c__2(...)`, causing javac to complain that `c(f2$type)` return type is incompatible with `y0$type`.

JADX 1.5.1 single-class `qr.k0` emits the interface shape as:

```java
@Override // nr.i1
public y0 c(f2 f2Var) { ... }
```

## Backend path

- Current alias allocation: `crates/decomp/src/java_aliases.rs`, `AliasInventory::build`, method family loop around return-only collision assignment.
- It sorts `MethodKey` by `(owner, name, parameters, return_type)` and assigns the first same-name/same-params method the base name. It has no access flag, bridge/synthetic, or hierarchy/interface obligation data.
- Existing related hook: `JavaAliases::align_final_bridge_targets` in `crates/decomp/src/java_aliases_bridge.rs`; current seed only handles a narrow zero-argument final bridge family and does not cover parameterized interface/source override collisions.
- Called from `crates/decomp/src/java_integration.rs` after `original_hierarchy` is built.

## Minimal fix direction

Add a post-build alias alignment pass, not stubs:

1. For each input owner and `(name, parameters)` family with return-only collisions, collect method access flags and ancestor/interface declarations from input/external metadata.
2. Select the source-obligated method that must keep the base Java name:
   - exact match to an implemented/overridden public/protected/package declaration, or
   - non-bridge/non-synthetic/public method over private delegate or synthetic bridge when the latter is not a valid Java source implementation.
3. Atomically swap/assign aliases so the selected method keeps the original source name and private/synthetic bridge methods get suffixed aliases.
4. Fail closed on ambiguous multiple source obligations or unknown hierarchy.
5. Add fixtures for:
   - private delegate + public interface getter (`SamsungPassServiceImpl` shape),
   - public bridge synthetic + public real interface method with same params and different returns (`qr/k0` shape),
   - ambiguous two public non-bridge declarations stays unchanged/rejected.

Expected risk reduction: removes a repeated class of real emitted-source compile errors without inventing declarations or accepting missing bodies.
