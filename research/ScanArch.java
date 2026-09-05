import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.Address;
import ghidra.program.model.block.*;

public class ScanArch extends GhidraScript {
  public void run() throws Exception {
    long baseCode = 0x83000L;
    long endCode  = 0x8c7b0L;
    Address start = toAddr(baseCode);
    try { disassemble(start); } catch (Exception e) { println("disasm err: "+e); }
    try { createFunction(start, "entry_guess"); } catch (Exception e) {}
    // seed a handful more candidate starts across the region to encourage flow discovery
    for (long a = baseCode; a < endCode; a += 0x200) {
      Address x = toAddr(a);
      if (currentProgram.getListing().getInstructionAt(x) == null) {
        try { disassemble(x); } catch (Exception e) {}
      }
    }
    analyzeAll(currentProgram);

    FunctionManager fm = currentProgram.getFunctionManager();
    FunctionIterator it = fm.getFunctions(true);
    int count = 0; long totalLen = 0;
    while (it.hasNext()) {
      Function f = it.next();
      long a = f.getEntryPoint().getOffset();
      if (a >= baseCode && a < endCode) {
        count++;
        totalLen += f.getBody().getNumAddresses();
      }
    }
    println("FUNC_COUNT=" + count + " AVG_LEN=" + (count>0? (totalLen/count) : 0));

    // instruction coverage in region
    Listing listing = currentProgram.getListing();
    long good=0, bad=0;
    Address a = start;
    Address end = toAddr(endCode);
    while (a.compareTo(end) < 0) {
      Instruction ins = listing.getInstructionAt(a);
      if (ins != null) { good += ins.getLength(); a = a.add(ins.getLength()); }
      else { bad++; a = a.add(1); }
    }
    println("COVERED_BYTES=" + good + " UNCOVERED_BYTES=" + bad);

    // decompile first 5 functions in region for manual sanity read
    DecompInterface d = new DecompInterface();
    d.openProgram(currentProgram);
    it = fm.getFunctions(true);
    int shown = 0;
    while (it.hasNext() && shown < 5) {
      Function f = it.next();
      long a2 = f.getEntryPoint().getOffset();
      if (a2 < baseCode || a2 >= endCode) continue;
      DecompileResults r = d.decompileFunction(f, 30, monitor);
      println("\n//=== " + f.getName() + " @ " + f.getEntryPoint() + " len=" + f.getBody().getNumAddresses() + " ===");
      if (r != null && r.decompileCompleted()) {
        println(r.getDecompiledFunction().getC());
      } else {
        println("// decompile failed");
      }
      shown++;
    }
  }
}
