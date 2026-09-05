import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.Address;

public class ListFuncs extends GhidraScript {
  public void run() throws Exception {
    // seed more of the code region to widen coverage before listing
    long baseCode = 0x83000L;
    long endCode  = 0x8c7b0L;
    for (long a = baseCode; a < endCode; a += 0x40) {
      Address x = toAddr(a);
      if (currentProgram.getListing().getInstructionAt(x) == null
          && currentProgram.getListing().getDefinedDataAt(x) == null) {
        try { disassemble(x); } catch (Exception e) {}
      }
    }
    analyzeAll(currentProgram);

    FunctionManager fm = currentProgram.getFunctionManager();
    FunctionIterator it = fm.getFunctions(true);
    int count = 0;
    long covered = 0;
    while (it.hasNext()) {
      Function f = it.next();
      long a = f.getEntryPoint().getOffset();
      if (a >= baseCode && a < endCode) {
        count++;
        long len = f.getBody().getNumAddresses();
        covered += len;
        println(String.format("FUNC 0x%x %s len=%d", a, f.getName(), len));
      }
    }
    println("TOTAL_FUNCS=" + count + " TOTAL_COVERED=" + covered + " REGION=" + (endCode-baseCode));

    // report byte coverage gaps (undisassembled runs > 16 bytes) for manual review
    Listing listing = currentProgram.getListing();
    Address a = toAddr(baseCode);
    Address end = toAddr(endCode);
    Address gapStart = null;
    while (a.compareTo(end) < 0) {
      boolean defined = listing.getInstructionAt(a) != null || listing.getDefinedDataAt(a) != null;
      if (!defined) {
        if (gapStart == null) gapStart = a;
      } else {
        if (gapStart != null) {
          long glen = a.subtract(gapStart);
          if (glen > 16) println(String.format("GAP 0x%x - 0x%x (len=%d)", gapStart.getOffset(), a.getOffset(), glen));
          gapStart = null;
        }
      }
      a = a.add(1);
    }
    if (gapStart != null) {
      long glen = end.subtract(gapStart);
      if (glen > 16) println(String.format("GAP 0x%x - 0x%x (len=%d)", gapStart.getOffset(), end.getOffset(), glen));
    }
  }
}
