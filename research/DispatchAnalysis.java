// DispatchAnalysis.java — headless Ghidra post-script for the JA11 dispatcher RE task.
//
// 1. Finds callers/references to entry_guess (0x83000) and FUN_ram_00083010 (0x83010).
// 2. Decompiles a fixed list of functions of interest (dispatcher + helpers).
// 3. Dumps disassembly context of the dispatcher switch tables.
// 4. Prints info about the ".gpr"-external low-address helper functions (func_0x00016c14 etc.)
//    to determine if they're external/unresolved or resident in-image.
//
// Usage (against the private copy, NOT the shared project):
//   analyzeHeadless /tmp/ja11-re-dispatch JA11_V2.2 -process JA11_V2.2.bin \
//       -readOnly -noanalysis -scriptPath /tmp/ja11-re-dispatch -postScript DispatchAnalysis.java

import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.Address;
import ghidra.program.model.symbol.*;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;

public class DispatchAnalysis extends GhidraScript {
    DecompInterface d;
    java.util.HashSet<Long> done = new java.util.HashSet<>();
    FunctionManager fm;
    ReferenceManager rm;
    SymbolTable st;

    Function fAt(long a) {
        return fm.getFunctionContaining(toAddr(a));
    }

    void dec(long addr, String tag) {
        Function f = fAt(addr);
        if (f == null) {
            println("// [" + tag + "] no function containing 0x" + Long.toHexString(addr));
            return;
        }
        long off = f.getEntryPoint().getOffset();
        if (!done.add(off)) {
            println("// [" + tag + "] " + f.getName() + " @ " + f.getEntryPoint() + " (already printed)");
            return;
        }
        DecompileResults r = d.decompileFunction(f, 120, monitor);
        println("\n//###### " + tag + " : " + f.getName() + " @ " + f.getEntryPoint() + " ######");
        println(r != null && r.decompileCompleted() ? r.getDecompiledFunction().getC() : "// <decompile failed>");
    }

    void showRefsTo(long addr, String label) {
        println("\n==== References TO 0x" + Long.toHexString(addr) + " (" + label + ") ====");
        Address a = toAddr(addr);
        ReferenceIterator it = rm.getReferencesTo(a);
        int n = 0;
        while (it.hasNext()) {
            Reference r = it.next();
            n++;
            Address from = r.getFromAddress();
            Function callerFn = fm.getFunctionContaining(from);
            println("  ref from " + from + " type=" + r.getReferenceType()
                    + " inFunc=" + (callerFn != null ? callerFn.getName() + "@" + callerFn.getEntryPoint() : "?"));
        }
        if (n == 0) println("  (none found via ReferenceManager)");
    }

    void checkSymbol(String name) {
        println("\n==== Symbol lookup: " + name + " ====");
        SymbolIterator it = st.getSymbols(name);
        boolean any = false;
        while (it.hasNext()) {
            Symbol s = it.next();
            any = true;
            Address a = s.getAddress();
            println("  symbol " + s.getName() + " @ " + a + " type=" + s.getSymbolType()
                    + " isExternal=" + s.isExternal() + " source=" + s.getSource());
            Function f = fm.getFunctionAt(a);
            if (f != null) {
                println("    -> function, body size=" + f.getBody().getNumAddresses()
                        + " thunk=" + f.isThunk() + " external=" + f.isExternal());
            }
            MemoryBlock blk = currentProgram.getMemory().getBlock(a);
            println("    -> memory block: " + (blk != null ? blk.getName() + " [" + blk.getStart() + "-" + blk.getEnd() + "] initialized=" + blk.isInitialized() : "NONE (address not in any block!)"));
        }
        if (!any) println("  (symbol not found by exact name)");
    }

    public void run() throws Exception {
        d = new DecompInterface();
        DecompileOptions opts = new DecompileOptions();
        d.setOptions(opts);
        d.openProgram(currentProgram);
        fm = currentProgram.getFunctionManager();
        rm = currentProgram.getReferenceManager();
        st = currentProgram.getSymbolTable();

        Memory mem = currentProgram.getMemory();
        println("=== Memory blocks ===");
        for (MemoryBlock b : mem.getBlocks()) {
            println("  " + b.getName() + " " + b.getStart() + "-" + b.getEnd()
                    + " init=" + b.isInitialized() + " exec=" + b.isExecute() + " read=" + b.isRead());
        }

        // Step 1: who calls the dispatcher entry points?
        showRefsTo(0x83000L, "entry_guess");
        showRefsTo(0x83010L, "FUN_ram_00083010");

        // Step 1b: check whether the low helper addresses even exist as real code in this image,
        // or are Ghidra placeholder/external symbols for addresses outside the loaded block.
        checkSymbol("func_0x00024b4c");
        checkSymbol("func_0x00016c14");
        checkSymbol("func_0x00016c00");
        checkSymbol("func_0x000154f8");

        // Step 2: decompile the dispatcher + everything it calls.
        long[] seeds = {
            0x83000L,  // entry_guess
            0x83010L,  // FUN_ram_00083010 (plain-C twin)
            0x84290L,  // FUN_ram_00084290
            0x84400L,  // FUN_ram_00084400
            0x84408L,  // FUN_ram_00084408
            0x84410L,  // (next func after 84408, might be related)
            0x85e54L,  // FUN_ram_00085e54 (cmd 9 handler)
            0x24b4cL,  // func_0x00024b4c (reply framer) -- may be outside loaded region
            0x16c14L,  // func_0x00016c14
            0x16c00L,  // func_0x00016c00
            0x154f8L,  // func_0x000154f8
        };
        for (long s : seeds) dec(s, "seed_0x" + Long.toHexString(s));

        // Step 3: decompile any callees of the dispatcher/helpers not already covered, one level deep.
        long[] followUps = {0x84370L, 0x842ecL, 0x842b4L, 0x84674L, 0x845f8L, 0x844fcL};
        for (long s : followUps) dec(s, "followup_0x" + Long.toHexString(s));

        println("\n=== DISPATCH ANALYSIS DONE ===");
    }
}
