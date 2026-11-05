// Thermal Eye sensor helper.
//
// LibreHardwareMonitor is the only maintained library that knows the register
// layouts of current CPUs, Super I/O chips and DIMM sensors, and it is .NET, so
// this small process owns the hardware access. Everything else is Rust.
//
// Streams newline-delimited JSON into a named pipe created by the UI. It runs
// elevated, so it deliberately reads nothing from the pipe: the only input it
// takes is its command line. It exits as soon as the UI goes away.
using System.IO.Pipes;
using System.Security.Principal;
using System.Text;
using System.Text.Json;
using LibreHardwareMonitor.Hardware;

return Helper.Run(args);

static class Helper
{
    // The UI has no use for the rest (timings, SMART counters, voltages, ...).
    static readonly HashSet<SensorType> Wanted =
    [
        SensorType.Temperature, SensorType.Load, SensorType.Clock, SensorType.Power,
        SensorType.Fan, SensorType.Data, SensorType.SmallData,
    ];

    public static int Run(string[] args)
    {
        string? Arg(string name)
        {
            int i = Array.IndexOf(args, name);
            return i >= 0 && i + 1 < args.Length ? args[i + 1] : null;
        }

        if (args.Contains("--register-task")) return TaskSetup.Register();
        if (args.Contains("--unregister-task")) return TaskSetup.Unregister();
        if (Arg("--dump") is { } dumpPath) return Dump(dumpPath);
        if (Arg("--record") is { } recordPath) return Record(recordPath, int.TryParse(Arg("--seconds"), out var s) ? s : 60);
        if (Arg("--pipe") is not { } pipeName) return 2;
        int interval = int.TryParse(Arg("--interval"), out var ms) ? Math.Clamp(ms, 250, 10000) : 1000;

        try
        {
            // Anonymous: whoever owns the pipe must not be able to borrow this
            // process's elevated identity.
            using var pipe = new NamedPipeClientStream(
                ".", pipeName, PipeDirection.Out, PipeOptions.None, TokenImpersonationLevel.Anonymous);
            pipe.Connect(15000);
            var computer = Open();
            try { Stream(pipe, computer, interval); }
            finally { computer.Close(); }
            return 0;
        }
        catch (IOException) { return 0; }     // UI closed the pipe.
        catch (TimeoutException) { return 3; }
    }

    static Computer Open()
    {
        var computer = new Computer
        {
            IsCpuEnabled = true, IsGpuEnabled = true, IsMemoryEnabled = true,
            IsMotherboardEnabled = true, IsStorageEnabled = true,
        };
        computer.Open();
        return computer;
    }

    static void Update(IHardware hardware)
    {
        hardware.Update();
        foreach (var sub in hardware.SubHardware) Update(sub);
    }

    static IEnumerable<IHardware> Flatten(IEnumerable<IHardware> list) =>
        list.SelectMany(h => new[] { h }.Concat(Flatten(h.SubHardware)));

    static void Stream(Stream pipe, Computer computer, int interval, int maxFrames = int.MaxValue)
    {
        bool admin = new WindowsPrincipal(WindowsIdentity.GetCurrent()).IsInRole(WindowsBuiltInRole.Administrator);
        var buffer = new MemoryStream(32 * 1024);
        string signature = "";
        List<ISensor> sensors = [];
        var clock = System.Diagnostics.Stopwatch.StartNew();
        long next = 0;

        for (int frame = 0; frame < maxFrames; frame++)
        {
            foreach (var h in computer.Hardware) Update(h);

            var hardware = Flatten(computer.Hardware).ToList();
            var current = hardware.SelectMany(h => h.Sensors).Where(s => Wanted.Contains(s.SensorType)).ToList();
            var currentSignature = string.Join('|', current.Select(s => s.Identifier.ToString()));

            buffer.SetLength(0);
            using (var json = new Utf8JsonWriter(buffer))
            {
                if (currentSignature != signature)
                {
                    signature = currentSignature;
                    sensors = current;
                    WriteSchema(json, admin, hardware, sensors);
                    json.Flush();
                    buffer.WriteByte((byte)'\n');
                    json.Reset();
                }
                WriteValues(json, sensors);
            }
            buffer.WriteByte((byte)'\n');
            pipe.Write(buffer.GetBuffer(), 0, (int)buffer.Length);
            pipe.Flush();

            next += interval;
            long wait = next - clock.ElapsedMilliseconds;
            if (wait > 0) Thread.Sleep((int)wait);
            else next = clock.ElapsedMilliseconds;   // Fell behind (e.g. resumed from sleep).
        }
    }

    static void WriteSchema(Utf8JsonWriter json, bool admin, List<IHardware> hardware, List<ISensor> sensors)
    {
        json.WriteStartObject();
        json.WriteString("t", "schema");
        json.WriteBoolean("admin", admin);
        json.WriteStartArray("hardware");
        foreach (var h in hardware)
        {
            json.WriteStartObject();
            json.WriteString("id", h.Identifier.ToString());
            json.WriteString("name", h.Name);
            json.WriteString("type", h.HardwareType.ToString());
            json.WriteEndObject();
        }
        json.WriteEndArray();
        json.WriteStartArray("sensors");
        foreach (var s in sensors)
        {
            json.WriteStartObject();
            json.WriteString("id", s.Identifier.ToString());
            json.WriteString("hw", s.Hardware.Identifier.ToString());
            json.WriteString("name", s.Name);
            json.WriteString("kind", s.SensorType.ToString());
            json.WriteEndObject();
        }
        json.WriteEndArray();
        json.WriteEndObject();
    }

    static void WriteValues(Utf8JsonWriter json, List<ISensor> sensors)
    {
        json.WriteStartObject();
        json.WriteString("t", "values");
        json.WriteStartArray("v");
        foreach (var s in sensors)
        {
            if (s.Value is float v && float.IsFinite(v)) json.WriteNumberValue(MathF.Round(v, 2));
            else json.WriteNullValue();
        }
        json.WriteEndArray();
        json.WriteEndObject();
    }

    // Development: captures the same stream to a file, for `thermal-eye --replay <file>`.
    static int Record(string path, int seconds)
    {
        using var file = File.Create(path);
        var computer = Open();
        try { Stream(file, computer, 1000, seconds); }
        finally { computer.Close(); }
        return 0;
    }

    // Diagnostic: `thermal-eye-sensors --dump <file>` lists everything the library sees.
    static int Dump(string path)
    {
        using var w = new StreamWriter(path, false, new UTF8Encoding(false));
        var computer = Open();
        foreach (var h in computer.Hardware) Update(h);
        Thread.Sleep(1500);
        foreach (var h in computer.Hardware) Update(h);
        foreach (var h in Flatten(computer.Hardware))
        {
            w.WriteLine($"[{h.HardwareType}] {h.Name} id={h.Identifier}");
            foreach (var s in h.Sensors)
                w.WriteLine($"  {s.SensorType,-12} {s.Name,-36} {s.Value?.ToString("0.##") ?? "null"}  id={s.Identifier}");
        }
        computer.Close();
        return 0;
    }
}
