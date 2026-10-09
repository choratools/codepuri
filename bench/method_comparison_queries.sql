-- Run with sqlite3 /path/to/comparison.sqlite < bench/method_comparison_queries.sql
.headers on
.mode column

-- 1. Root DEX scope matrix. "No marker" is not a correctness result.
SELECT a.label, COALESCE(m.codepuri_status,'unobserved') AS codepuri,
       COALESCE(m.jadx_status,'unobserved') AS jadx, COUNT(*) AS methods
FROM method_comparison m JOIN apps a ON a.id=m.app_id
WHERE m.pool IS NOT NULL
GROUP BY a.id,m.codepuri_status,m.jadx_status;

-- 2. Additional/non-root DEX evidence, including undecodable asset entries.
SELECT a.label,d.entry,d.root,d.bytes,d.defined_methods,d.parse_error
FROM dex_entries d JOIN apps a ON a.id=d.app_id WHERE d.root=0;

-- 3. Largest methods explicitly unavailable in Codepuri, observed by JADX.
SELECT a.label,m.identity,m.code_units,m.try_count,m.features,m.codepuri_reason
FROM method_comparison m JOIN apps a ON a.id=m.app_id
WHERE m.codepuri_status IN ('explicitly_unavailable','unit_unpublished')
  AND m.jadx_status='body_observed_no_marker'
ORDER BY m.code_units DESC LIMIT 30;

-- 4. Bodies emitted by Codepuri that have explicit JADX diagnostic markers.
SELECT a.label,m.identity,m.code_units,m.jadx_reason
FROM method_comparison m JOIN apps a ON a.id=m.app_id
WHERE m.codepuri_status IN ('body_emitted','body_emitted_with_diagnostic')
  AND m.jadx_status='diagnostic_marked'
ORDER BY m.code_units DESC LIMIT 30;

-- 5. Cross-app code fingerprint candidates, excluding tiny trivial methods.
SELECT symbolic_code_sha256,COUNT(DISTINCT app_id) AS apps,COUNT(*) AS methods,
       MIN(identity) AS example,MAX(code_units) AS code_units
FROM methods WHERE symbolic_code_sha256 IS NOT NULL AND code_units>=8
GROUP BY symbolic_code_sha256 HAVING COUNT(DISTINCT app_id)>1
ORDER BY apps DESC,code_units DESC LIMIT 30;

-- 6. Class-level hotspots. Do not attribute their time to individual methods.
SELECT a.label,c.descriptor,c.elapsed_seconds,c.method_count,c.stages
FROM class_profiles c JOIN apps a ON a.id=c.app_id
ORDER BY c.elapsed_seconds DESC LIMIT 30;

-- 7. Identity attribution quality and unparsed artifact evidence.
SELECT tool,match_basis,COUNT(*) AS records FROM observations GROUP BY tool,match_basis;
SELECT tool,reason,COUNT(*) AS records FROM unmatched GROUP BY tool,reason;

-- 8. Counts of actual validation records. Empty means unmeasured.
SELECT tool,kind,status,COUNT(*) FROM validations GROUP BY tool,kind,status;
