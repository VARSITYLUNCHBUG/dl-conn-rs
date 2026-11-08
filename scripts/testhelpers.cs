// Win32 helpers shared by the test scripts.
using System; using System.Text; using System.Drawing; using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c,string t);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,string l);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,StringBuilder l);
  [DllImport("user32")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32")] static extern bool SetCursorPos(int x,int y);
  [DllImport("user32")] static extern void mouse_event(uint f,uint x,uint y,uint d,UIntPtr e);
  public static void Move(int x,int y){SetCursorPos(x,y);}
  public static void Click(int x,int y){SetCursorPos(x,y);System.Threading.Thread.Sleep(80);mouse_event(2,0,0,0,UIntPtr.Zero);mouse_event(4,0,0,0,UIntPtr.Zero);}
  delegate bool EnumProc(IntPtr h,IntPtr l);
  [DllImport("user32")] static extern bool EnumWindows(EnumProc f,IntPtr l);
  [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
  public static IntPtr TitledWindow(int pid){IntPtr found=IntPtr.Zero;EnumWindows((h,l)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==pid&&IsWindowVisible(h)&&Title(h).StartsWith("dl-conn-rs")){found=h;return false;}return true;},IntPtr.Zero);return found;}
  [DllImport("user32")] static extern uint MapVirtualKey(uint code,uint map);
  public static void PostKey(IntPtr h,uint vk){uint scan=MapVirtualKey(vk,0);PostMessage(h,0x100,(IntPtr)vk,(IntPtr)(1|(scan<<16)));PostMessage(h,0x101,(IntPtr)vk,(IntPtr)(int)(1|(scan<<16)|0xC0000000));}
  public static int[] Rect(IntPtr h){RECT r;GetWindowRect(h,out r);return new[]{r.L,r.T,r.R,r.B};}
  [DllImport("user32")] public static extern IntPtr GetDlgItem(IntPtr h,int id);
  [DllImport("user32")] public static extern bool GetWindowRect(IntPtr h,out RECT r);
  [DllImport("user32")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint f);
  [StructLayout(LayoutKind.Sequential)] public struct RECT{public int L,T,R,B;}
  public static string Title(IntPtr h){var s=new StringBuilder(512);GetWindowText(h,s,512);return s.ToString();}
  public static void Shot(IntPtr h,string path){RECT r;GetWindowRect(h,out r);using(var b=new Bitmap(r.R-r.L,r.B-r.T))using(var g=Graphics.FromImage(b)){var dc=g.GetHdc();PrintWindow(h,dc,2);g.ReleaseHdc(dc);b.Save(path);}}
}
