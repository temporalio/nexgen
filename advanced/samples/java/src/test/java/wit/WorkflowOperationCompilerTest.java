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
          String name =
              (procedure ? "void" : "value")
                  + " workflow arity "
                  + arity
                  + ", signal arity "
                  + (signalArgument ? 1 : 0);
          tests.add(check(name, call(arity, procedure, signalArgument), true));
        }
      }
    }
    tests.add(check("workflow arity seven", call(7, false, true), false));
    tests.add(check("void workflow arity seven", call(7, true, true), false));
    tests.add(
        check(
            "signal arity two",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\", \"id\","
                + " \"queue\", t::signalTwo, \"a\", \"b\");",
            false));
    tests.add(
        check(
            "workflow missing argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"id\", \"queue\","
                + " t::signal, \"signal\");",
            false));
    tests.add(
        check(
            "workflow excess argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\","
                + " \"extra\", \"id\", \"queue\", t::signal, \"signal\");",
            false));
    tests.add(
        check(
            "workflow wrong type",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, 42, \"id\","
                + " \"queue\", t::signal, \"signal\");",
            false));
    tests.add(
        check(
            "signal missing argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\", \"id\","
                + " \"queue\", t::signal);",
            false));
    tests.add(
        check(
            "signal excess argument",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\", \"id\","
                + " \"queue\", t::signal, \"signal\", \"extra\");",
            false));
    tests.add(
        check(
            "signal wrong type",
            "Target1 t=null; WorkflowOperations.signalWithStartWorkflow(t::run, \"input\", \"id\","
                + " \"queue\", t::signal, 42);",
            false));
    tests.add(
        check(
            "wrong generic workflow result",
            "Target1 t=null; WorkflowOperations.<String, String,"
                + " Integer>signalWithStartWorkflow(t::run, \"input\", \"id\", \"queue\","
                + " t::signal, \"signal\");",
            false));
    tests.add(
        check(
            "correct generic workflow result",
            "Target1 t=null; WorkflowOperations.<String, String,"
                + " String>signalWithStartWorkflow(t::run, \"input\", \"id\", \"queue\", t::signal,"
                + " \"signal\");",
            true));
    tests.add(
        check(
            "valid nested generic arguments",
            "Generic t=null; WorkflowOperations.signalWithStartWorkflow(t::run,"
                + " Collections.singletonList(\"input\"), \"id\", \"queue\", t::signal,"
                + " Collections.singletonMap(\"key\", 1));",
            true));
    tests.add(
        check(
            "wrong nested workflow generic argument",
            "Generic t=null; WorkflowOperations.signalWithStartWorkflow(t::run,"
                + " Collections.singletonList(42), \"id\", \"queue\", t::signal,"
                + " Collections.singletonMap(\"key\", 1));",
            false));
    tests.add(
        check(
            "wrong nested signal generic argument",
            "Generic t=null; WorkflowOperations.signalWithStartWorkflow(t::run,"
                + " Collections.singletonList(\"input\"), \"id\", \"queue\", t::signal,"
                + " Collections.singletonMap(\"key\", \"bad\"));",
            false));
    for (boolean options : new boolean[] {false, true}) {
      tests.add(
          check(
              "untyped higher arities, options=" + options,
              "WorkflowOperations.signalWithStartWorkflow(\"type\", new Object[]{1,2,3,4,5,6,7},"
                  + " \"id\", \"queue\", \"signal\", new Object[]{1,2}"
                  + (options ? ", SignalWithStartWorkflowOptions.getDefaultInstance()" : "")
                  + ");",
              true));
    }
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
    return tests;
  }

  private DynamicTest check(String name, String body, boolean expected) {
    return DynamicTest.dynamicTest(
        name,
        () -> {
          JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
          assertNotNull(compiler, "Compiler tests require a JDK");
          String code = declarations() + "void check() { " + body + " } }";
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
    source
        .append("\"id\", \"queue\", ")
        .append(signalArgument ? "t::signal, \"signal\"" : "t::signalZero");
    if (arity % 2 == 0) source.append(", SignalWithStartWorkflowOptions.getDefaultInstance()");
    return source.append(");").toString();
  }
}
