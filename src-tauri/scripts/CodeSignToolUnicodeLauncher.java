import com.ssl.code.signing.tool.CodeSignTool;
import java.nio.charset.StandardCharsets;
import java.util.Base64;

public final class CodeSignToolUnicodeLauncher {
    private static final String PROGRAM_NAME_PREFIX = "--unicode-program-name-base64=";

    private CodeSignToolUnicodeLauncher() {}

    public static void main(String[] args) {
        if (args.length == 0 || !args[0].startsWith(PROGRAM_NAME_PREFIX)) {
            System.err.println("Missing Base64-encoded Unicode program name");
            System.exit(2);
        }

        String encodedProgramName = args[0].substring(PROGRAM_NAME_PREFIX.length());
        String programName = new String(
            Base64.getDecoder().decode(encodedProgramName),
            StandardCharsets.UTF_8
        );
        String[] forwarded = new String[args.length];
        System.arraycopy(args, 1, forwarded, 0, args.length - 1);
        forwarded[forwarded.length - 1] = "-program_name=" + programName;
        CodeSignTool.main(forwarded);
    }
}
