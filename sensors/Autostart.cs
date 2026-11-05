// One-time setup that lets the widget start the helper without a UAC prompt.
//
// A scheduled task can run elevated on demand, but whatever it points at must not
// be writable by unelevated programs, or any of them could swap the file and gain
// admin rights silently. So the task only ever runs a copy in Program Files.
using System.Diagnostics;
using System.Security;
using System.Security.Principal;
using System.Text;

static class Autostart
{
    const string TaskName = "Thermal Eye Sensors";
    const string PipeName = "thermal-eye-sensors";
    const string ExeName = "thermal-eye-sensors.exe";

    static string InstallDir =>
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "ThermalEye");

    static string InstalledExe => Path.Combine(InstallDir, ExeName);

    static bool IsInstalledCopy =>
        string.Equals(Environment.ProcessPath, InstalledExe, StringComparison.OrdinalIgnoreCase);

    public static int Install()
    {
        Directory.CreateDirectory(InstallDir);
        CopySelf();

        // The definition stays inside the protected folder: schtasks reads it
        // elevated, so it must not sit anywhere another program could rewrite it.
        var definition = Path.Combine(InstallDir, "task.xml");
        File.WriteAllText(definition, TaskXml(), Encoding.Unicode);
        try { return Schtasks("/create", "/f", "/tn", TaskName, "/xml", definition); }
        finally { File.Delete(definition); }
    }

    public static int Remove()
    {
        int result = TaskExists() ? Schtasks("/delete", "/f", "/tn", TaskName) : 0;
        DeleteInstallDir();
        return result;
    }

    // Called on every elevated start from outside Program Files. That is the only
    // moment the installed copy is sure to be idle, so this finishes a removal the
    // running helper blocked, and refreshes the copy after an update.
    public static void Sync()
    {
        if (IsInstalledCopy || !Directory.Exists(InstallDir)) return;
        try
        {
            if (TaskExists()) CopySelf();
            else DeleteInstallDir();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException) { }
    }

    static void CopySelf()
    {
        if (IsInstalledCopy) return;
        var self = Environment.ProcessPath!;
        if (File.Exists(InstalledExe) && File.ReadAllBytes(self).AsSpan().SequenceEqual(File.ReadAllBytes(InstalledExe)))
            return;
        StopInstalledCopies();
        File.Copy(self, InstalledExe, overwrite: true);
    }

    static void StopInstalledCopies()
    {
        foreach (var process in Process.GetProcessesByName(Path.GetFileNameWithoutExtension(ExeName)))
        {
            using (process)
            {
                if (process.Id == Environment.ProcessId) continue;
                try
                {
                    if (!string.Equals(process.MainModule?.FileName, InstalledExe, StringComparison.OrdinalIgnoreCase)) continue;
                    process.Kill();
                    process.WaitForExit(3000);
                }
                catch (Exception e) when (e is InvalidOperationException or System.ComponentModel.Win32Exception) { }
            }
        }
    }

    // The helper may still be running from here; it is then cleaned up by Sync.
    static void DeleteInstallDir()
    {
        try { if (Directory.Exists(InstallDir)) Directory.Delete(InstallDir, recursive: true); }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException) { }
    }

    static bool TaskExists() => Schtasks("/query", "/tn", TaskName) == 0;

    static int Schtasks(params string[] arguments)
    {
        // A full path: a bare name would be searched for beside this exe first,
        // and that folder is writable by unelevated programs.
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
    static string TaskXml()
    {
        var user = WindowsIdentity.GetCurrent().User!.Value;
        var exe = SecurityElement.Escape(InstalledExe);
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
                  <Command>{exe}</Command>
                  <Arguments>--pipe {PipeName}</Arguments>
                </Exec>
              </Actions>
            </Task>
            """;
    }
}
