import java.nio.file.*;
import java.util.*;
import javax.tools.*;
import com.sun.source.util.JavacTask;

/** Parse exact generated files without attribution or fabricated dependencies. */
public class SamsungPassParseGate {
    public static void main(String[] args) throws Exception {
        String release = args.length > 1 ? args[1] : "17";
        if (!release.equals("17") && !release.equals("25")) {
            throw new IllegalArgumentException("source release must be 17 or 25");
        }
        List<Path> paths;
        try (var stream = Files.walk(Path.of(args[0]))) {
            paths = stream.filter(p -> p.toString().endsWith(".java")).sorted().toList();
        }
        var compiler = ToolProvider.getSystemJavaCompiler();
        long errors = 0;
        // Bound memory, not diagnostics: every original source is parsed.
        try (var files = compiler.getStandardFileManager(null, null, null)) {
            for (int start = 0; start < paths.size(); start += 500) {
                var diagnostics = new DiagnosticCollector<JavaFileObject>();
                var task = (JavacTask) compiler.getTask(null, files, diagnostics,
                    List.of("--release", release, "-proc:none", "-Xmaxerrs", "2147483647"), null,
                    files.getJavaFileObjectsFromPaths(paths.subList(start, Math.min(start + 500, paths.size()))));
                for (var tree : task.parse()) { }
                for (var diagnostic : diagnostics.getDiagnostics()) {
                    if (diagnostic.getKind() == Diagnostic.Kind.ERROR) {
                        errors++;
                        System.err.println(diagnostic);
                    }
                }
            }
        }
        System.out.println("{\"units\":" + paths.size() + ",\"syntax_errors\":" + errors + "}");
        if (errors != 0) System.exit(1);
    }
}
