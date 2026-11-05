// Win32 helpers shared by the elevated test scripts: drive the app's windows, and attach to its
// console to send real keystrokes and read the screen back.
using System; using System.Text; using System.Drawing; using System.Runtime.InteropServices; using Microsoft.Win32.SafeHandles;
public static class W {
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c,string t);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p,IntPtr a,string c,string t);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,string l);
  [DllImport("user32",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,StringBuilder l);
  [DllImport("user32")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32")] public static extern bool GetWindowRect(IntPtr h,out RECT r);
  [DllImport("user32")] public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint f);
  [DllImport("user32")] public static extern IntPtr GetSystemMenu(IntPtr h,bool revert);
  [DllImport("user32")] public static extern int GetMenuState(IntPtr m,uint id,uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT{public int L,T,R,B;}
  public static string Title(IntPtr h){var s=new StringBuilder(512);GetWindowText(h,s,512);return s.ToString();}
  public static string Selected(IntPtr c){var i=SendMessage(c,0x147,IntPtr.Zero,IntPtr.Zero);var s=new StringBuilder(256);SendMessage(c,0x148,i,s);return s.ToString();}
  public static void Shot(IntPtr h,string path){RECT r;GetWindowRect(h,out r);using(var b=new Bitmap(r.R-r.L,r.B-r.T))using(var g=Graphics.FromImage(b)){var dc=g.GetHdc();PrintWindow(h,dc,2);g.ReleaseHdc(dc);b.Save(path);}}
}
public static class Con {
  [DllImport("kernel32")] static extern bool FreeConsole();
  [DllImport("kernel32")] static extern bool AllocConsole();
  [DllImport("kernel32")] static extern bool AttachConsole(uint pid);
  [DllImport("kernel32")] public static extern IntPtr GetConsoleWindow();
  [DllImport("user32")] static extern bool ShowWindow(IntPtr h,int cmd);
  [DllImport("kernel32",CharSet=CharSet.Unicode,SetLastError=true)] static extern SafeFileHandle CreateFile(string n,uint a,uint s,IntPtr sa,uint d,uint f,IntPtr t);
  [DllImport("kernel32",EntryPoint="WriteConsoleInputW")] static extern bool WriteInput(SafeFileHandle h,Rec[] r,uint n,out uint w);
  [DllImport("kernel32",EntryPoint="ReadConsoleOutputCharacterW",CharSet=CharSet.Unicode)] static extern bool ReadChars(SafeFileHandle h,StringBuilder s,uint n,Coord c,out uint r);
  [DllImport("kernel32",EntryPoint="GetConsoleScreenBufferInfo")] static extern bool Info(SafeFileHandle h,out Buf b);
  [StructLayout(LayoutKind.Sequential)] struct Coord{public short X,Y;}
  [StructLayout(LayoutKind.Sequential)] struct Buf{public Coord Size,Cursor;public ushort Attr;public short L,T,R,B;public Coord Max;}
  [StructLayout(LayoutKind.Explicit,Size=20)] struct Rec{[FieldOffset(0)]public ushort Type;[FieldOffset(4)]public int Down;[FieldOffset(8)]public ushort Repeat;[FieldOffset(10)]public ushort Vk;[FieldOffset(12)]public ushort Scan;[FieldOffset(14)]public ushort Ch;[FieldOffset(16)]public uint Ctrl;}
  static SafeFileHandle Open(string n){var h=CreateFile(n,0xC0000000,3,IntPtr.Zero,3,0,IntPtr.Zero);if(h.IsInvalid)throw new Exception(n+" error "+Marshal.GetLastWin32Error());return h;}
  public static bool Attach(uint pid){FreeConsole();return AttachConsole(pid);}
  public static void Detach(){FreeConsole();AllocConsole();ShowWindow(GetConsoleWindow(),0);}
  public static void Type(string text){using(var h=Open("CONIN$")){foreach(char c in text){ushort vk=(ushort)(c=='\t'?9:c=='\r'?13:c==(char)27?27:0);var recs=new Rec[2];for(int i=0;i<2;i++){recs[i].Type=1;recs[i].Down=i==0?1:0;recs[i].Repeat=1;recs[i].Vk=vk;recs[i].Ch=(ushort)c;}uint w;WriteInput(h,recs,2,out w);}}}
  public static string[] Rows(){using(var h=Open("CONOUT$")){Buf b;Info(h,out b);var rows=new string[b.Cursor.Y+1];for(int y=0;y<=b.Cursor.Y;y++){var s=new StringBuilder(b.Size.X+1);uint r;var c=new Coord();c.Y=(short)y;ReadChars(h,s,(uint)b.Size.X,c,out r);rows[y]=s.ToString(0,(int)r).TrimEnd();}return rows;}}
}
