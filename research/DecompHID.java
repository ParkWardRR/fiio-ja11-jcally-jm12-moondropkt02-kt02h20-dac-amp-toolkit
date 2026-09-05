// DecompHID.java — Ghidra headless post-script used to decompile the JA11 Upgrade Tool's
// HID/flash command functions to C. Java (not Python) because headless Ghidra here has no
// PyGhidra bridge — a .py post-script errors with "Python is not available".
//
// Usage:
//   analyzeHeadless <projDir> <projName> -import "JadeAudio JA11 Upgrade Tool.exe" \
//       -scriptPath . -postScript DecompHID.java
// Re-run against a saved project without re-analysing:
//   analyzeHeadless <projDir> <projName> -process "JadeAudio JA11 Upgrade Tool.exe" \
//       -noanalysis -scriptPath . -postScript DecompHID.java
//
// Edit `seeds` to the addresses you want (rizin and Ghidra share the PE ImageBase 0x400000).

import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.Address;
import ghidra.program.model.symbol.*;

public class DecompHID extends GhidraScript {
    DecompInterface d;
    java.util.HashSet<Long> done = new java.util.HashSet<>();
    FunctionManager fm;

    Function ensure(long a) {
        Address x = toAddr(a);
        Function f = fm.getFunctionContaining(x);
        if (f == null) {
            try { disassemble(x); f = createFunction(x, null); } catch (Exception e) {}
        }
        return f;
    }

    void dec(Function f, String tag) {
        if (f == null) { println("// no func " + tag); return; }
        long off = f.getEntryPoint().getOffset();
        if (!done.add(off)) return;
        DecompileResults r = d.decompileFunction(f, 90, monitor);
        println("\n//###### " + tag + " : " + f.getName() + " @ " + f.getEntryPoint() + " ######");
        println(r != null && r.decompileCompleted() ? r.getDecompiledFunction().getC() : "// <decompile failed>");
    }

    public void run() throws Exception {
        d = new DecompInterface();
        d.openProgram(currentProgram);
        fm = currentProgram.getFunctionManager();
        ReferenceManager rm = currentProgram.getReferenceManager();

        // Command builders + the "UpGrade" orchestrator (addresses for this exact .exe build).
        long[] seeds = {
            0x54d0e0L, // status/handshake (cmd 0x33)
            0x54d330L, // read word       (cmd 0x08)
            0x6246c0L, // erase           (cmd 0x21)
            0x5583a0L, // write word      (cmd 0x88)
            0x6260b0L, // unlock "T12345678" (report 0x54)
            0x624680L, // connect: hid_open(vid,pid)
            0xb8ed20L, // orchestrator: connect -> unlock -> flash worker
        };
        for (long s : seeds) dec(ensure(s), "seed");

        // Also decompile callers of the unlock, to find the orchestrator generically.
        Function unlock = ensure(0x6260b0L);
        if (unlock != null)
            for (Reference r : rm.getReferencesTo(unlock.getEntryPoint()))
                dec(fm.getFunctionContaining(r.getFromAddress()), "caller-of-unlock");

        println("\n=== DECOMP DONE ===");
    }
}
