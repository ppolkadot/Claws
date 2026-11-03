param([Parameter(Mandatory)][string]$Out, [switch]$KeepPattern)
# Verifies the real window material: puts a known test pattern directly behind Claws,
# lifts Claws above it, and captures only Claws' own rectangle. Nothing else on the
# user's screen can end up in the image.
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
if (-not ("GS" -as [type])) {
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class GS {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int s);
  // Windows above the target in z-order that are visible, not cloaked and overlap its rect.
  public static int OverlapsAbove(IntPtr target, RECT r) {
    int n = 0;
    for (IntPtr w = GetWindow(target, 3); w != IntPtr.Zero; w = GetWindow(w, 3)) {
      if (!IsWindowVisible(w)) continue;
      int cloaked; if (DwmGetWindowAttribute(w, 14, out cloaked, 4) == 0 && cloaked != 0) continue;
      RECT o; GetWindowRect(w, out o);
      if (o.R <= r.L || o.L >= r.R || o.B <= r.T || o.T >= r.B || o.R - o.L <= 0 || o.B - o.T <= 0) continue;
      n++;
    }
    return n;
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static IntPtr Find(uint pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid) { var sb = new StringBuilder(64); GetClassName(h, sb, 64); if (sb.ToString() == "Tauri Window") { found = h; return false; } }
      return true; }, IntPtr.Zero);
    return found;
  }
}
"@
}
[GS]::SetProcessDPIAware() | Out-Null
$proc = Get-Process -Name claws | Sort-Object StartTime -Descending | Select-Object -First 1
$h = [GS]::Find([uint32]$proc.Id)
if ($h -eq [IntPtr]::Zero) { throw "Tauri window not found" }
if ([GS]::IsIconic($h)) { [GS]::ShowWindow($h, 4) | Out-Null; Start-Sleep -Milliseconds 600 }
$r = New-Object GS+RECT; [GS]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $ht = $r.B - $r.T

# The test pattern: bold colour bars, a white field, a black field and large text,
# so blur, tint and transparency are all easy to judge.
$form = New-Object System.Windows.Forms.Form
$form.FormBorderStyle = 'None'; $form.StartPosition = 'Manual'; $form.TopMost = $true; $form.ShowInTaskbar = $false
$form.Bounds = New-Object System.Drawing.Rectangle(($r.L - 40), ($r.T - 40), ($w + 80), ($ht + 80))
$form.Add_Paint({
  param($s, $e)
  $g = $e.Graphics; $W = $s.ClientSize.Width; $H = $s.ClientSize.Height
  $colors = @('#e8453c','#f39c12','#f1c40f','#2ecc71','#1abc9c','#3498db','#9b59b6','#ffffff','#111111')
  $bw = [math]::Ceiling($W / $colors.Count)
  for ($i = 0; $i -lt $colors.Count; $i++) {
    $b = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($colors[$i]))
    $g.FillRectangle($b, $i * $bw, 0, $bw, $H); $b.Dispose()
  }
  $font = New-Object System.Drawing.Font('Segoe UI', 64, [System.Drawing.FontStyle]::Bold)
  $g.DrawString('DESKTOP TEST PATTERN', $font, [System.Drawing.Brushes]::Black, 60, ($H / 2 - 60))
  $g.DrawString('DESKTOP TEST PATTERN', $font, [System.Drawing.Brushes]::White, 56, ($H / 2 - 64))
  $font.Dispose()
})
$form.Show()
1..10 | ForEach-Object { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 40 }

# Lift Claws above the pattern, capture its rectangle, then put the order back.
$TOPMOST = [IntPtr](-1); $NOTOPMOST = [IntPtr](-2); $FLAGS = 0x0001 -bor 0x0002 -bor 0x0010  # NOSIZE|NOMOVE|NOACTIVATE
[GS]::SetWindowPos($h, $TOPMOST, 0, 0, 0, 0, $FLAGS) | Out-Null
1..20 | ForEach-Object { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 40 }
# Privacy guard: never save a capture if any other window overlaps Claws.
$blocking = [GS]::OverlapsAbove($h, $r)
if ($blocking -gt 0) {
  [GS]::SetWindowPos($h, $NOTOPMOST, 0, 0, 0, 0, $FLAGS) | Out-Null
  $form.Close(); $form.Dispose()
  "ABORTED: $blocking other window(s) overlap Claws; nothing was captured"
  return
}
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$g.Dispose()
if ([GS]::OverlapsAbove($h, $r) -gt 0) { $bmp.Dispose(); "ABORTED: a window moved in front during capture; discarded"; [GS]::SetWindowPos($h, $NOTOPMOST, 0, 0, 0, 0, $FLAGS) | Out-Null; $form.Close(); return }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
[GS]::SetWindowPos($h, $NOTOPMOST, 0, 0, 0, 0, $FLAGS) | Out-Null
if (-not $KeepPattern) { $form.Close(); $form.Dispose() }
"saved ${w}x${ht}"
