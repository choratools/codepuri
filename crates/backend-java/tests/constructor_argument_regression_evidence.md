# Constructor argument evaluation regressions

This records the audit before replacing allocating String reconstruction with native String constants. It does not claim that absent native-literal improvements are measured recovery.

## Wave 11: 138 constructor identities withdrawn

Every original prefix allocates and initializes one or more objects before loading its String constant. Every exact previously emitted constructor instead evaluates `new java.lang.String(new char[] {...}).intern()` before the object arguments. The audit compared all original selected code items and all original-identity source declarations using ledger paths, including aliases.

Independent production-resolver witnesses:

- classes3 #42458, `JCEKeyGenerator$ARIA`: CipherKeyGenerator allocation at code-unit 0, initializer at 2, String at 5.
- classes3 #42680, `JDKDSASigner$KCDSA`: HAS160Digest allocation/initializer at 0/2, KCDSASigner at 5/7, String at 10.
- #17552, `DSTU7564$KeyGenerator256`: CipherKeyGenerator at 0/2, String at 5.

The actual backend rejected `pre-chain argument order/escape`. Caller delegation was correctly `None`; this was not superclass-target delegation metadata failure.

Full raw/source audit: `/tmp/opencode/wave11-constructor-loss-order.json`.

## Wave 12: 169 withdrawn identities

Of these, 116 are constructors and 53 are static initializers. All 53 static-initializer identities have an unavailable constructor in the same original owner and are reported as required-body closure failures.

Selected constructor code items and their exact old source declarations establish:

| Original constructor population | Count | Finding |
|---|---:|---|
| Changed literal order | 111 | ClassLiteral/String loads and allocating expressions differ in order |
| Duplicated literal in chain arguments | 1 | `androidx.room.e1$a` loads one empty String, but old header constructs/interns it twice |
| Duplicated literal after chain | 4 | Four BouncyCastle constructors reuse one original String for field stores; old source constructs/interns identical UTF-16 strings again |

For the four post-chain cases, the original physical register was tracked through later definitions and copies; only stores retaining the original String identity were counted. A reused register overwritten with a numeric/null value was excluded. The same old UTF-16 expression appears at least twice within each exact constructor declaration.

- DSTU KeyPairGeneratorSpi: original String store at code-unit 15, field 8929.
- GM BaseSM2: stores at 28/32, fields 8964/8965.
- EC KeyPairGeneratorSpi$EC: stores at 28/32, fields 9020/9021.
- ECGOST12 KeyPairGeneratorSpi: store at 15, field 9087.

Full raw/source audit: `/tmp/opencode/wave12-constructor-loss-order.json`.

## Proof boundary and future native constants

Fresh String/char-array construction is not equivalent to loading a String constant: it adds allocations and may add failures. Native Java literals can preserve exact interned String references without those explicit allocations. [JVMS 5.1](https://docs.oracle.com/javase/specs/jvms/se17/html/jvms-5.html#jvms-5.1) defines String constants as static references derived from constant-pool entries; [JLS 3.10.5](https://docs.oracle.com/javase/specs/jls/se17/html/jls-3.html#jls-3.10.5) defines literal interning. Unicode translation precedes lexical processing, so escaping must satisfy [JLS 3.3](https://docs.oracle.com/javase/specs/jls/se17/html/jls-3.html#jls-3.3), including unpaired UTF-16 surrogates and literal backslash-u sequences.

The bounded constructor helper currently retains exact literal-event order, single-use allocating expressions, complete-arena reads and original receiver/chain proofs. Native-literal repeatability must be enabled only with verified constant-pool reuse and no explicit NEW/String.intern instructions. It must not erase ClassLiteral ordering or fabricate bridge/carrier allocations.

Regression tests: `constructor_argument_allocations.rs`. No full-APK export or application execution was performed for this audit.
