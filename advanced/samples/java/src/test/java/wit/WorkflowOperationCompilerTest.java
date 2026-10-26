package wit;

import static org.junit.jupiter.api.Assertions.*;

import java.net.URI;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import javax.tools.*;
import org.junit.jupiter.api.DynamicTest;
import org.junit.jupiter.api.TestFactory;
import org.junit.jupiter.api.io.TempDir;

/** Compile public calls against freshly generated sources, including calls that must fail. */
class WorkflowOperationCompilerTest {
  @TempDir Path output;

  @TestFactory
  List<DynamicTest> boundMethodsAndUntypedCalls() {
    List<DynamicTest> tests = new ArrayList<>();
    for (int arity = 0; arity <= 6; arity++) {
      for (boolean procedure : new boolean[] {false, true}) {
        for (boolean signalArgument : new boolean[] {false, true}) {
          tests.add(
              check(
                  (procedure ? "void" : "value")
                      + " workflow arity "
                      + arity
                      + ", signal arity "
                      + (signalArgument ? 1 : 0),
                  call(arity, procedure, signalArgument),
                  true));
        }
      }
    }
    tests.add(check("workflow arity seven", call(7, false, true), false));
    tests.add(check("void workflow arity seven", call(7, true, true), false));
    tests.add(
        check(
            "signal arity two",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\","
                + " t::signalTwo, \"a\", \"b\", options);",
            false));
    tests.add(
        check(
            "workflow missing argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, t::signal,"
                + " \"signal\", options);",
            false));
    tests.add(
        check(
            "workflow excess argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\","
                + " \"extra\", t::signal, \"signal\", options);",
            false));
    tests.add(
        check(
            "workflow wrong type",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, 42, t::signal,"
                + " \"signal\", options);",
            false));
    tests.add(
        check(
            "signal missing argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\","
                + " t::signal, options);",
            false));
    tests.add(
        check(
            "signal excess argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\","
                + " t::signal, \"signal\", \"extra\", options);",
            false));
    tests.add(
        check(
            "signal wrong type",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\","
                + " t::signal, 42, options);",
            false));
    tests.add(
        check(
            "wrong generic workflow result",
            "Target1 t=null; WorkflowOperations.<String, String,"
                + " Integer>signalWithStartWorkflow(t::run, \"input\", t::signal, \"signal\","
                + " options);",
            false));
    tests.add(
        check(
            "correct generic workflow result",
            "Target1 t=null; WorkflowOperations.<String, String,"
                + " String>signalWithStartWorkflow(t::run, \"input\", t::signal, \"signal\","
                + " options);",
            true));
    tests.add(
        check(
            "valid nested generic arguments",
            "Generic t=null; WorkflowOperations.signalWithStartWorkflow(t::run,"
                + " Collections.singletonList(\"input\"), t::signal,"
                + " Collections.singletonMap(\"key\", 1), options);",
            true));
    tests.add(
        check(
            "wrong nested workflow generic argument",
            "Generic t=null; WorkflowOperations.signalWithStartWorkflow(t::run,"
                + " Collections.singletonList(42), t::signal, Collections.singletonMap(\"key\", 1),"
                + " options);",
            false));
    tests.add(
        check(
            "wrong nested signal generic argument",
            "Generic t=null; WorkflowOperations.signalWithStartWorkflow(t::run,"
                + " Collections.singletonList(\"input\"), t::signal,"
                + " Collections.singletonMap(\"key\", \"bad\"), options);",
            false));
    tests.add(
        check(
            "untyped higher arities",
            "WorkflowOperations.signalWithStartWorkflow(\"type\", new Object[]{1,2,3,4,5,6,7},"
                + " \"signal\", new Object[]{1,2}, options);",
            true));
    tests.add(
        check(
            "typed options are required",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\","
                + " t::signal, \"signal\");",
            false));
    tests.add(
        check(
            "zero-arity typed options are required",
            "Target0 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, t::signalZero);",
            false));
    tests.add(
        check(
            "untyped options are required",
            "WorkflowOperations.signalWithStartWorkflow(\"type\", new Object[0], \"signal\", new"
                + " Object[0]);",
            false));
    tests.add(
        check(
            "ID and task queue cannot be top-level arguments",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\", \"id\","
                + " \"queue\", t::signal, \"signal\", options);",
            false));
    tests.add(
        check(
            "workflow arguments cannot be options",
            "SignalWithStartWorkflowOptions.newBuilder().setArgs(\"input\");",
            false));
    tests.add(
        check(
            "signal arguments cannot be options",
            "SignalWithStartWorkflowOptions.newBuilder().setSignalArgs(\"signal\");",
            false));
    tests.add(
        check(
            "generic operation takes only options",
            "wit.genericmodels.CompleteOptions<String> genericOptions = null;"
                + " wit.genericmodels.GenericModelServiceOperations.complete(genericOptions);",
            true));
    tests.add(
        check(
            "generic result type cannot be a top-level argument",
            "wit.genericmodels.CompleteOptions<String> genericOptions = null;"
                + " wit.genericmodels.GenericModelServiceOperations.complete(Object.class,"
                + " genericOptions);",
            false));
    return tests;
  }

  private DynamicTest check(String name, String body, boolean expected) {
    return DynamicTest.dynamicTest(
        name,
        () -> {
          JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
          assertNotNull(compiler, "Compiler tests require a JDK");
          String code =
              declarations()
                  + "void check() { SignalWithStartWorkflowOptions options ="
                  + " SignalWithStartWorkflowOptions.newBuilder().setId(\"id\").setTaskQueue(\"queue\").build();"
                  + " "
                  + body
                  + " } }";
          JavaFileObject source =
              new SimpleJavaFileObject(
                  URI.create("string:///CompilerCase.java"), JavaFileObject.Kind.SOURCE) {
                @Override
                public CharSequence getCharContent(boolean ignored) {
                  return code;
                }
              };
          DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
          try (StandardJavaFileManager files =
              compiler.getStandardFileManager(diagnostics, null, null)) {
            boolean compiled =
                compiler
                    .getTask(
                        null,
                        files,
                        diagnostics,
                        Arrays.asList(
                            "-proc:none",
                            "--release",
                            "8",
                            "-classpath",
                            System.getProperty("nexgen.test.classpath"),
                            "-d",
                            output.toString()),
                        null,
                        Arrays.asList(source))
                    .call();
            assertEquals(expected, compiled, () -> name + ": " + diagnostics.getDiagnostics());
          }
        });
  }

  private static String declarations() {
    StringBuilder source =
        new StringBuilder(
            "import io.temporal.workflow.*; import wit.workflowservice.WorkflowOperations; import"
                + " wit.workflowservice.SignalWithStartWorkflowOptions; import java.util.*; public"
                + " class CompilerCase {");
    for (int arity = 0; arity <= 7; arity++) {
      for (boolean procedure : new boolean[] {false, true}) {
        source
            .append("@WorkflowInterface public interface ")
            .append(procedure ? "Void" : "Target")
            .append(arity)
            .append(" { @WorkflowMethod ")
            .append(procedure ? "void" : "String")
            .append(" run(");
        for (int i = 0; i < arity; i++)
          source.append(i == 0 ? "" : ", ").append("String a").append(i);
        source.append(
            "); @SignalMethod void signal(String value); @SignalMethod void signalZero();"
                + " @SignalMethod void signalTwo(String a, String b); }");
      }
    }
    return source
        .append(
            "@WorkflowInterface public interface Generic { @WorkflowMethod List<Integer>"
                + " run(List<String> input); @SignalMethod void signal(Map<String, Integer> input);"
                + " }")
        .toString();
  }

  private static String call(int arity, boolean procedure, boolean signalArgument) {
    StringBuilder source =
        new StringBuilder(
            (procedure ? "Void" : "Target")
                + arity
                + " t=null; WorkflowOperations.signalWithStartWorkflow(t::run, ");
    for (int i = 0; i < arity; i++) source.append("\"arg").append(i).append("\", ");
    source.append(signalArgument ? "t::signal, \"signal\"" : "t::signalZero");
    return source.append(", options);").toString();
  }
}
