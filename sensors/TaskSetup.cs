// Registers the scheduled task that lets the widget start this helper elevated,
// with no UAC prompt. The installer calls this once.
//
// The task runs as administrator without asking, so it must never point at a file
// an unelevated program could replace, or that program would gain admin rights
// silently. Registration therefore refuses to run from outside Program Files.
using System.Diagnostics;
using System.Security;
using System.Security.Principal;
using System.Text;

static class TaskSetup
{
    const string TaskName = "Thermal Eye Sensors";
    const string PipeName = "thermal-eye-sensors";
    const int NotInProgramFiles = 4;

    public static int Register()
    {
        var exe = Environment.ProcessPath!;
        var protectedRoot = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles) + Path.DirectorySeparatorChar;
        if (!exe.StartsWith(protectedRoot, StringComparison.OrdinalIgnoreCase)) return NotInProgramFiles;

        // Beside the exe, for the same reason: schtasks reads this file elevated.
        var definition = Path.Combine(Path.GetDirectoryName(exe)!, "task.xml");
        File.WriteAllText(definition, TaskXml(exe), Encoding.Unicode);
        try { return Schtasks("/create", "/f", "/tn", TaskName, "/xml", definition); }
        finally { File.Delete(definition); }
    }

    public static int Unregister() =>
        Schtasks("/query", "/tn", TaskName) == 0 ? Schtasks("/delete", "/f", "/tn", TaskName) : 0;

    static int Schtasks(params string[] arguments)
    {
        // A full path: a bare name would be searched for beside this exe first.
        var start = new ProcessStartInfo(Path.Combine(Environment.SystemDirectory, "schtasks.exe"))
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var argument in arguments) start.ArgumentList.Add(argument);
        using var process = Process.Start(start)!;
        process.StandardOutput.ReadToEnd();
        process.StandardError.ReadToEnd();
        process.WaitForExit();
        return process.ExitCode;
    }

    // No triggers: the widget starts it on demand. The time limit is off because
    // tasks are otherwise stopped after three days.
    static string TaskXml(string exe)
    {
        var user = WindowsIdentity.GetCurrent().User!.Value;
        return $"""
            <?xml version="1.0" encoding="UTF-16"?>
            <Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
              <RegistrationInfo><Description>Reads hardware sensors for the Thermal Eye widget.</Description></RegistrationInfo>
              <Principals>
                <Principal id="Author">
                  <UserId>{user}</UserId>
                  <LogonType>InteractiveToken</LogonType>
                  <RunLevel>HighestAvailable</RunLevel>
                </Principal>
              </Principals>
              <Settings>
                <MultipleInstancesPolicy>StopExisting</MultipleInstancesPolicy>
                <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
                <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
                <AllowHardTerminate>true</AllowHardTerminate>
                <AllowStartOnDemand>true</AllowStartOnDemand>
                <Enabled>true</Enabled>
                <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
                <Priority>6</Priority>
              </Settings>
              <Actions Context="Author">
                <Exec>
                  <Command>{SecurityElement.Escape(exe)}</Command>
                  <Arguments>--pipe {PipeName}</Arguments>
                </Exec>
              </Actions>
            </Task>
            """;
    }
}
