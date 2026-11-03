param([Parameter(Mandatory)][string]$OutPrefix)
# Control experiment, independent of Tauri: does this PC render DWM blur at all?
# A plain WinForms window gets the same accent blur and system Acrylic calls the app
# uses, over a known test pattern; only its own rectangle is captured.
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class BP {
  [StructLayout(LayoutKind.Sequential)] public struct MARGINS { public int l, r, t, b; }
  [StructLayout(LayoutKind.Sequential)] public struct ACCENT { public int state, flags, color, anim; }
  [StructLayout(LayoutKind.Sequential)] public struct WCADATA { public int attr; public IntPtr data; public IntPtr size; }
  [DllImport("dwmapi.dll")] public static extern int DwmExtendFrameIntoClientArea(IntPtr h, ref MARGINS m);
  [DllImport("dwmapi.dll")] public static extern int DwmSetWindowAttribute(IntPtr h, int a, ref int v, int s);
  [DllImport("dwmapi.dll")] public static extern int DwmIsCompositionEnabled(out bool e);
  [DllImport("user32.dll")] public static extern int SetWindowCompositionAttribute(IntPtr h, ref WCADATA d);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public static int Accent(IntPtr h, int state, int flags, int color) {
    var a = new ACCENT { state = state, flags = flags, color = color, anim = 0 };
    int size = Marshal.SizeOf(a); IntPtr p = Marshal.AllocHGlobal(size); Marshal.StructureToPtr(a, p, false);
    var d = new WCADATA { attr = 19, data = p, size = (IntPtr)size };
    int r = SetWindowCompositionAttribute(h, ref d); Marshal.FreeHGlobal(p); return r;
  }
}
"@
[BP]::SetProcessDPIAware() | Out-Null
$comp = $false; [BP]::DwmIsCompositionEnabled([ref]$comp) | Out-Null
"DWM composition enabled: $comp"

$x = 200; $y = 160; $w = 640; $h = 360
$pattern = New-Object System.Windows.Forms.Form
$pattern.FormBorderStyle = 'None'; $pattern.StartPosition = 'Manual'; $pattern.TopMost = $true; $pattern.ShowInTaskbar = $false
$pattern.Bounds = New-Object System.Drawing.Rectangle(($x - 40), ($y - 40), ($w + 80), ($h + 80))
$pattern.Add_Paint({ param($s, $e)
  $g = $e.Graphics; $W = $s.ClientSize.Width; $H = $s.ClientSize.Height
  $cs = @('#e8453c','#f1c40f','#2ecc71','#3498db','#9b59b6','#ffffff','#111111'); $bw = [math]::Ceiling($W / $cs.Count)
  for ($i = 0; $i -lt $cs.Count; $i++) { $b = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($cs[$i])); $g.FillRectangle($b, $i * $bw, 0, $bw, $H); $b.Dispose() }
  $f = New-Object System.Drawing.Font('Segoe UI', 40, [System.Drawing.FontStyle]::Bold); $g.DrawString('SHARP TEXT', $f, [System.Drawing.Brushes]::Black, 40, 150); $f.Dispose() })
$pattern.Show()

function Capture($name) {
  1..15 | ForEach-Object { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 40 }
  $bmp = New-Object System.Drawing.Bitmap $w, $h; $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size); $g.Dispose(); $bmp.Save("$OutPrefix-$name.png"); $bmp.Dispose()
}

foreach ($mode in 'accent-blur', 'system-acrylic') {
  $f = New-Object System.Windows.Forms.Form
  $f.FormBorderStyle = 'None'; $f.StartPosition = 'Manual'; $f.TopMost = $true; $f.ShowInTaskbar = $false
  $f.Bounds = New-Object System.Drawing.Rectangle($x, $y, $w, $h); $f.BackColor = [System.Drawing.Color]::Black
  $f.Show(); [System.Windows.Forms.Application]::DoEvents()
  $m = New-Object BP+MARGINS; $m.l = -1; $m.r = -1; $m.t = -1; $m.b = -1
  $r1 = [BP]::DwmExtendFrameIntoClientArea($f.Handle, [ref]$m)
  if ($mode -eq 'accent-blur') { $r2 = [BP]::Accent($f.Handle, 3, 0, 0) } else { $v = 3; $r2 = [BP]::DwmSetWindowAttribute($f.Handle, 38, [ref]$v, 4) }
  "$mode : extend=0x{0:x} apply=0x{1:x}" -f $r1, $r2
  Capture $mode
  $f.Close(); $f.Dispose()
}
$pattern.Close(); $pattern.Dispose()
