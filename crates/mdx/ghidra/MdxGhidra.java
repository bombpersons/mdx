// Mega Drive memory setup, runtime-evidence seeding, index export and bounded
// function decompilation for `mdx ghidra`. The Rust supervisor computes every
// address (vectors, header, hardware blocks, evidence) and passes a JSON request
// file; this script only applies it. Stdout is reserved for Ghidra logs.
// @category MegaDrive
import com.google.gson.*;
import ghidra.app.util.headless.HeadlessScript;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.cmd.function.CreateFunctionCmd;
import ghidra.program.disassemble.Disassembler;
import ghidra.app.decompiler.DecompInterface;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.pcode.JumpTable;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.ArrayList;

public class MdxGhidra extends HeadlessScript {
    private final Gson gson = new GsonBuilder().setPrettyPrinting().create();
    private JsonObject request;
    private final JsonArray warnings = new JsonArray();

    private String hex(Address a) { return String.format("0x%08X", a.getOffset()); }
    private Address at(JsonElement e) { return toAddr(e.getAsLong()); }
    private void write(String key, JsonObject value) throws Exception {
        try (var out = Files.newBufferedWriter(Path.of(request.get(key).getAsString()), StandardCharsets.UTF_8)) {
            gson.toJson(value, out);
        }
    }
    private JsonObject function(Function f) {
        var out = new JsonObject();
        out.addProperty("entry", hex(f.getEntryPoint()));
        out.addProperty("name", f.getName());
        out.addProperty("name_source", f.getSymbol().getSource().toString());
        out.addProperty("signature", f.getSignature().toString());
        out.addProperty("comment", f.getComment());
        var body = new JsonArray();
        for (var range : f.getBody()) {
            var r = new JsonObject();
            r.addProperty("start", hex(range.getMinAddress()));
            r.addProperty("end_inclusive", hex(range.getMaxAddress()));
            body.add(r);
        }
        out.add("body", body);
        return out;
    }
    /** Marks [lo, hi) executable, splitting blocks so neighbouring data stays non-executable. */
    private void executable(Memory memory, Address lo, Address hi) throws Exception {
        var part = memory.getBlock(lo);
        if (part == null) throw new IOException("no block at " + hex(lo));
        if (!part.getStart().equals(lo)) { memory.split(part, lo); part = memory.getBlock(lo); }
        if (part.getEnd().compareTo(hi) > 0) memory.split(part, hi.add(1));
        part.setExecute(true);
    }
    private void setup() throws Exception {
        var memory = currentProgram.getMemory();
        var main = memory.getBlock(at(request.get("base")));
        main.setName(request.get("main_block").getAsString());
        main.setRead(true);
        main.setWrite(request.get("main_writable").getAsBoolean());
        main.setExecute(false);
        for (var item : request.getAsJsonArray("blocks")) {
            var b = item.getAsJsonObject();
            var start = at(b.get("start"));
            long size = b.get("size").getAsLong();
            String name = b.get("name").getAsString();
            MemoryBlock block;
            if (b.has("mapped_to")) {
                block = memory.createByteMappedBlock(name, start, at(b.get("mapped_to")), size, false);
            } else if (b.has("file")) {
                try (var in = Files.newInputStream(Path.of(b.get("file").getAsString()))) {
                    block = memory.createInitializedBlock(name, start, in, size, monitor, false);
                }
            } else {
                block = memory.createUninitializedBlock(name, start, size, false);
            }
            block.setRead(true);
            block.setWrite(b.get("writable").getAsBoolean());
            block.setExecute(false);
            block.setVolatile(b.get("volatile").getAsBoolean());
            if (b.has("comment")) block.setComment(b.get("comment").getAsString());
        }
        var code = new AddressSet();
        for (var item : request.getAsJsonArray("code_ranges")) {
            var r = item.getAsJsonArray();
            var lo = toAddr(r.get(0).getAsLong());
            var hi = toAddr(r.get(1).getAsLong() - 1);
            code.add(lo, hi);
            executable(memory, lo, hi);
        }
        // Register context, e.g. a base register held constant by compiled code.
        for (var item : request.getAsJsonArray("registers")) {
            var r = item.getAsJsonObject();
            var reg = currentProgram.getRegister(r.get("name").getAsString());
            if (reg == null) { warnings.add("unknown register " + r.get("name").getAsString()); continue; }
            for (var range : code) {
                currentProgram.getProgramContext().setValue(reg, range.getMinAddress(), range.getMaxAddress(),
                    java.math.BigInteger.valueOf(r.get("value").getAsLong()));
            }
        }
        // Without this, analysis follows bad pointers into I/O, RAM and data.
        currentProgram.getOptions(Program.DISASSEMBLER_PROPERTIES).setBoolean(
            Disassembler.RESTRICT_DISASSEMBLY_TO_EXECUTE_MEMORY_PROPERTY, true);
        // Typed data first, so the vector table and header are never disassembled.
        for (var item : request.getAsJsonArray("data")) {
            var d = item.getAsJsonObject();
            var a = at(d.get("address"));
            try {
                switch (d.get("type").getAsString()) {
                    case "dword": createDWord(a); break;
                    case "word": createWord(a); break;
                    case "ascii": createAsciiString(a, d.get("length").getAsInt()); break;
                    default: createByte(a);
                }
                if (d.has("name")) createLabel(a, d.get("name").getAsString(), true, SourceType.IMPORTED);
            } catch (Exception e) {
                warnings.add("data " + hex(a) + ": " + e.getMessage());
            }
        }
        for (var item : request.getAsJsonArray("labels")) {
            var l = item.getAsJsonObject();
            var a = at(l.get("address"));
            try {
                createLabel(a, l.get("name").getAsString(), true, SourceType.IMPORTED);
            } catch (Exception e) {
                warnings.add("label " + hex(a) + ": " + e.getMessage());
            }
        }
        for (var item : request.getAsJsonArray("entries")) {
            var entry = at(item);
            currentProgram.getSymbolTable().addExternalEntryPoint(entry);
            new DisassembleCommand(entry, code, true).applyTo(currentProgram, monitor);
            if (getFunctionAt(entry) == null && createFunction(entry, null) == null)
                warnings.add("entry " + hex(entry) + ": no function could be created");
        }
        for (var item : request.getAsJsonArray("symbols")) {
            monitor.checkCancelled();
            var s = item.getAsJsonObject();
            var a = at(s.get("address"));
            try {
                if (s.get("kind").getAsString().equals("function")) {
                    new DisassembleCommand(a, code, true).applyTo(currentProgram, monitor);
                    var f = getFunctionAt(a);
                    if (f == null) f = createFunction(a, null);
                    if (f == null) throw new IOException("function overlaps existing code");
                    f.setName(s.get("name").getAsString(), s.has("source") && s.get("source").getAsString().equals("imported") ? SourceType.IMPORTED : SourceType.USER_DEFINED);
                } else {
                    // Primary, so pseudocode reads the name instead of DAT_/Ram addresses.
                    createLabel(a, s.get("name").getAsString(), true, SourceType.USER_DEFINED);
                    sized(a, s);
                }
            } catch (Exception e) {
                warnings.add("symbol " + hex(a) + ": " + e.getMessage());
            }
        }
        // Runtime evidence: every executed PC is an instruction start.
        var executed = new AddressSet();
        for (var item : request.getAsJsonArray("executed")) {
            var a = at(item);
            if (code.contains(a)) executed.add(a);
        }
        if (!executed.isEmpty()) new DisassembleCommand(executed, code, true).applyTo(currentProgram, monitor);
        int rejected = 0;
        for (var item : request.getAsJsonArray("executed")) {
            if (getInstructionAt(at(item)) == null) rejected++;
        }
        // Observed indirect transfers: only computed jumps/calls gain references.
        int added = 0, ignored = 0;
        for (var item : request.getAsJsonArray("flows")) {
            monitor.checkCancelled();
            var pair = item.getAsJsonArray();
            var from = toAddr(pair.get(0).getAsLong());
            var to = toAddr(pair.get(1).getAsLong());
            var ins = getInstructionAt(from);
            if (ins == null || !code.contains(to)) { ignored++; continue; }
            var flow = ins.getFlowType();
            if (!flow.isComputed() || !(flow.isJump() || flow.isCall())) { ignored++; continue; }
            ins.addOperandReference(0, to, flow.isCall() ? RefType.COMPUTED_CALL : RefType.COMPUTED_JUMP, SourceType.USER_DEFINED);
            new DisassembleCommand(to, code, true).applyTo(currentProgram, monitor);
            if (flow.isCall() && getFunctionAt(to) == null) createFunction(to, null);
            added++;
        }
        var result = new JsonObject();
        result.add("warnings", warnings);
        result.addProperty("evidence_pcs_rejected", rejected);
        result.addProperty("evidence_flows_added", added);
        result.addProperty("evidence_flows_ignored", ignored);
        write("setup_result", result);
    }
    /** Turns observed computed-jump targets into decompiler switch overrides. */
    private int jumpTables() throws Exception {
        int written = 0;
        if (!request.has("jump_tables")) return 0;
        for (var item : request.getAsJsonArray("jump_tables")) {
            var t = item.getAsJsonObject();
            var from = at(t.get("from"));
            var f = getFunctionContaining(from);
            var ins = getInstructionAt(from);
            if (f == null || ins == null || !ins.getFlowType().isComputed() || !ins.getFlowType().isJump()) continue;
            var targets = new ArrayList<Address>();
            for (var to : t.getAsJsonArray("targets")) targets.add(at(to));
            try {
                new JumpTable(from, targets, true, 0).writeOverride(f);
                CreateFunctionCmd.fixupFunctionBody(currentProgram, f, monitor);
                written++;
            } catch (Exception e) {
                warnings.add("jump table " + hex(from) + ": " + e.getMessage());
            }
        }
        return written;
    }
    /**
     * `jsr d16(An)` / `jmp d16(An)` with An fixed by register context: Ghidra's
     * 68000 semantics load a pointer from An+d16 and call through it, so a
     * table of `jmp abs.l` stubs reads as calls to 0x4EF9xxxx. Resolve each
     * site to An+d16, make a `jmp abs.l` stub there a thunk of its target, and
     * override the call so pseudocode names the routine. Odd or non-code
     * targets are reported, never turned into functions.
     */
    private JsonObject baseRegisterCalls() throws Exception {
        var out = new JsonObject();
        int resolved = 0, thunks = 0;
        var rejected = new JsonArray();
        var executable = currentProgram.getMemory().getExecuteSet();
        var context = currentProgram.getProgramContext();
        var refs = currentProgram.getReferenceManager();
        var sites = new ArrayList<Instruction>();
        for (var it = currentProgram.getListing().getInstructions(true); it.hasNext();) {
            var ins = it.next();
            var m = ins.getMnemonicString().toLowerCase();
            if ((m.equals("jsr") || m.equals("jmp")) && ins.getFlowType().isComputed()) sites.add(ins);
        }
        for (var ins : sites) {
            monitor.checkCancelled();
            ghidra.program.model.lang.Register base = null;
            long disp = 0;
            boolean simple = true;
            for (var o : ins.getOpObjects(0)) {
                if (o instanceof ghidra.program.model.lang.Register r && base == null) base = r;
                else if (o instanceof ghidra.program.model.scalar.Scalar sc) disp = sc.getSignedValue();
                else simple = false;
            }
            // Only (An) and (d16,An); indexed forms vary at run time.
            if (!simple || base == null || !base.getName().matches("(?i)A[0-6]")) continue;
            var value = context.getRegisterValue(base, ins.getAddress());
            if (value == null || !value.hasValue()) continue;
            var target = toAddr((value.getUnsignedValue().longValue() + disp) & 0xFFFFFFL);
            boolean call = ins.getFlowType().isCall();
            String why = null;
            // A stub with a bad target still names the stub: that is what the CPU calls.
            boolean overrideAnyway = false;
            Address callee = target;
            if ((target.getOffset() & 1) != 0 || !executable.contains(target)) {
                why = "target is odd or outside code";
            } else {
                var stubTarget = jmpAbsLong(target);
                if (stubTarget != null) {
                    if ((stubTarget.getOffset() & 1) != 0 || !executable.contains(stubTarget)) {
                        why = "jmp stub at " + hex(target) + " leads to odd or non-code " + hex(stubTarget);
                        overrideAnyway = true;
                    }
                    else if (makeThunk(target, stubTarget)) thunks++;
                } else if (call) {
                    new DisassembleCommand(target, executable, true).applyTo(currentProgram, monitor);
                    if (getFunctionAt(target) == null && createFunction(target, null) == null) why = "no function could be created at " + hex(target);
                }
            }
            if (why != null) {
                if (rejected.size() < 256) {
                    var r = new JsonObject();
                    r.addProperty("from", hex(ins.getAddress()));
                    r.addProperty("target", hex(target));
                    r.addProperty("reason", why);
                    rejected.add(r);
                }
                if (!overrideAnyway) continue;
            }
            var type = call ? RefType.CALL_OVERRIDE_UNCONDITIONAL : RefType.JUMP_OVERRIDE_UNCONDITIONAL;
            for (var r : ins.getOperandReferences(0)) refs.delete(r);
            var ref = refs.addMemoryReference(ins.getAddress(), callee, type, SourceType.USER_DEFINED, 0);
            refs.setPrimary(ref, true);
            if (why == null) resolved++;
        }
        out.addProperty("resolved", resolved);
        out.addProperty("thunks_created", thunks);
        out.add("rejected", rejected);
        return out;
    }
    /** Target of a `jmp abs.l` (4EF9 xxxxxxxx) at a, or null. */
    private Address jmpAbsLong(Address a) {
        try {
            var mem = currentProgram.getMemory();
            if ((mem.getShort(a) & 0xFFFF) != 0x4EF9) return null;
            return toAddr(mem.getInt(a.add(2)) & 0xFFFFFFL);
        } catch (Exception e) {
            return null;
        }
    }
    /** Makes the stub at `stub` a thunk of the function at `target`; true if newly made. */
    private boolean makeThunk(Address stub, Address target) throws Exception {
        var existing = getFunctionAt(stub);
        if (existing != null && existing.isThunk()) return false;
        var code = currentProgram.getMemory().getExecuteSet();
        if (getInstructionAt(stub) == null) {
            clearListing(stub, stub.add(5));
            new DisassembleCommand(stub, code, false).applyTo(currentProgram, monitor);
        }
        new DisassembleCommand(target, code, true).applyTo(currentProgram, monitor);
        var callee = getFunctionAt(target);
        if (callee == null) callee = createFunction(target, null);
        if (callee == null) throw new IOException("no function at thunk target " + hex(target));
        if (existing != null) {
            existing.setThunkedFunction(callee);
        } else {
            var cmd = new ghidra.app.cmd.function.CreateThunkFunctionCmd(stub, new AddressSet(stub, stub.add(5)), target);
            if (!cmd.applyTo(currentProgram, monitor)) throw new IOException("thunk at " + hex(stub) + ": " + cmd.getStatusMsg());
        }
        return true;
    }
    /** Snapshot RAM is not a string table: drop data analysis invented there. */
    private int clearSnapshotData() throws Exception {
        int cleared = 0;
        if (!request.has("clear_data_blocks")) return 0;
        for (var name : request.getAsJsonArray("clear_data_blocks")) {
            var block = currentProgram.getMemory().getBlock(name.getAsString());
            if (block == null) continue;
            var data = new ArrayList<Data>();
            for (var d : currentProgram.getListing().getDefinedData(new AddressSet(block.getStart(), block.getEnd()), true)) data.add(d);
            for (var d : data) { removeData(d); cleared++; }
        }
        return cleared;
    }
    private void exportIndex() throws Exception {
        var index = new JsonObject();
        index.addProperty("snapshot_data_cleared", clearSnapshotData());
        index.addProperty("analysis_timed_out", analysisTimeoutOccurred());
        index.addProperty("jump_table_overrides", jumpTables());
        index.add("base_register_calls", baseRegisterCalls());
        var functions = new JsonArray();
        for (var f : currentProgram.getFunctionManager().getFunctions(true)) {
            monitor.checkCancelled();
            functions.add(function(f));
        }
        index.add("functions", functions);
        var refs = new JsonArray();
        var listing = currentProgram.getListing();
        long instructions = 0;
        for (var it = listing.getInstructions(true); it.hasNext();) {
            monitor.checkCancelled();
            var instruction = it.next();
            instructions++;
            for (var ref : instruction.getReferencesFrom()) {
                if (refs.size() >= 500000) break;
                var row = new JsonObject();
                row.addProperty("from", hex(ref.getFromAddress()));
                row.addProperty("to", hex(ref.getToAddress()));
                row.addProperty("to_space", ref.getToAddress().getAddressSpace().getName());
                row.addProperty("type", ref.getReferenceType().toString());
                row.addProperty("call", ref.getReferenceType().isCall());
                row.addProperty("source", ref.getSource().toString());
                var owner = getFunctionContaining(ref.getFromAddress());
                if (owner != null) row.addProperty("function", hex(owner.getEntryPoint()));
                refs.add(row);
            }
        }
        index.add("references", refs);
        index.addProperty("references_truncated", refs.size() >= 500000);
        index.addProperty("instructions", instructions);
        // How much observed execution the static model explains.
        var outside = new JsonArray();
        int inFunction = 0, noInstruction = 0, total = 0;
        if (request.has("executed")) {
            for (var item : request.getAsJsonArray("executed")) {
                var a = at(item);
                total++;
                if (getInstructionAt(a) == null) noInstruction++;
                else if (getFunctionContaining(a) != null) inFunction++;
                else if (outside.size() < 256) outside.add(hex(a));
            }
        }
        var evidence = new JsonObject();
        evidence.addProperty("executed_pcs", total);
        evidence.addProperty("in_functions", inFunction);
        evidence.addProperty("not_disassembled", noInstruction);
        evidence.add("outside_functions_sample", outside);
        index.add("evidence", evidence);
        if (mismatches != null) index.add("label_size_mismatches", mismatches);
        index.add("warnings", warnings);
        write("index", index);
    }
    /** Typed data for a named global, so wider accesses resolve to its name. */
    private void sized(Address a, JsonObject s) throws Exception {
        if (!s.has("size") || s.get("size").isJsonNull()) return;
        int size = s.get("size").getAsInt();
        clearListing(a, a.add(size - 1));
        switch (size) {
            case 1: createByte(a); break;
            case 2: createWord(a); break;
            default: createDWord(a);
        }
    }
    private void annotate() throws Exception {
        int transaction = currentProgram.startTransaction("mdx annotations");
        boolean committed = false;
        try {
            // Vacate all names first so swaps and generated-name displacement are deterministic.
            for (var item : request.getAsJsonArray("patches")) {
                var p = item.getAsJsonObject();
                var f = getFunctionAt(toAddr(p.get("address").getAsLong()));
                if (f == null) throw new IOException("annotation function entry missing");
                if (p.has("name")) f.setName(null, SourceType.DEFAULT);
            }
            for (var item : request.getAsJsonArray("patches")) {
                var p = item.getAsJsonObject();
                var f = getFunctionAt(toAddr(p.get("address").getAsLong()));
                if (p.has("name")) f.setName(p.get("name").getAsString(),
                    p.has("generated") ? SourceType.ANALYSIS : SourceType.USER_DEFINED);
                if (p.has("comment")) f.setComment(p.get("comment").getAsString());
            }
            if (request.has("labels")) {
                for (var item : request.getAsJsonArray("labels")) {
                    var l = item.getAsJsonObject();
                    var a = toAddr(l.get("address").getAsLong());
                    createLabel(a, l.get("name").getAsString(), true, SourceType.USER_DEFINED);
                    sized(a, l);
                    if (l.has("comment")) setEOLComment(a, l.get("comment").getAsString());
                }
            }
            committed = true;
        } finally { currentProgram.endTransaction(transaction, committed); }
        mismatches = request.has("labels") ? sizeMismatches(request.getAsJsonArray("labels")) : new JsonArray();
        exportIndex();
    }
    private JsonArray mismatches;
    /**
     * Sized labels the code accesses at another width (size 2, read by
     * `move.b`), which print as `name._0_1_`. Only sized 68000 mnemonics
     * (`.b/.w/.l`) are judged.
     */
    private JsonArray sizeMismatches(JsonArray labels) {
        var out = new JsonArray();
        var refs = currentProgram.getReferenceManager();
        for (var item : labels) {
            var l = item.getAsJsonObject();
            if (!l.has("size")) continue;
            int size = l.get("size").getAsInt();
            var a = toAddr(l.get("address").getAsLong());
            var sites = new java.util.TreeMap<Integer, JsonArray>();
            var counts = new java.util.TreeMap<Integer, Integer>();
            for (int off = 0; off < size; off++) {
                for (var ref : refs.getReferencesTo(a.add(off))) {
                    if (!ref.getReferenceType().isData()) continue;
                    var ins = getInstructionAt(ref.getFromAddress());
                    if (ins == null) continue;
                    var m = ins.getMnemonicString().toLowerCase();
                    int width = m.endsWith(".b") ? 1 : m.endsWith(".w") ? 2 : m.endsWith(".l") ? 4 : 0;
                    if (width == 0 || (width == size && off == 0)) continue;
                    counts.merge(width, 1, Integer::sum);
                    var row = sites.computeIfAbsent(width, k -> new JsonArray());
                    if (row.size() < 16) row.add(hex(ins.getAddress()) + " " + ins);
                }
            }
            for (var e : sites.entrySet()) {
                var r = new JsonObject();
                r.addProperty("name", l.get("name").getAsString());
                r.addProperty("address", hex(a));
                r.addProperty("size", size);
                r.addProperty("access_size", e.getKey());
                r.addProperty("accesses", counts.get(e.getKey()));
                r.add("sites", e.getValue());
                out.add(r);
            }
        }
        return out;
    }
    /** Decompiles each of request.addrs in one decompiler session. */
    private void decompile() throws Exception {
        var results = new JsonArray();
        var decompiler = new DecompInterface();
        try {
            if (!decompiler.openProgram(currentProgram)) throw new IOException(decompiler.getLastMessage());
            for (var item : request.getAsJsonArray("addrs")) {
                monitor.checkCancelled();
                results.add(decompileOne(decompiler, toAddr(item.getAsLong())));
            }
        } finally {
            decompiler.dispose();
        }
        var out = new JsonObject();
        out.add("results", results);
        write("query_result", out);
    }
    private JsonObject decompileOne(DecompInterface decompiler, Address a) throws Exception {
        var result = new JsonObject();
        var f = getFunctionContaining(a);
        if (f == null) {
            result.addProperty("status", "function_not_found");
            result.addProperty("diagnostic", "No analysed function contains this address; seed it with entries, symbols or runtime evidence.");
            return result;
        }
        result.add("function", function(f));
        // The instructions are the ground truth the pseudocode must be checked against.
        var listing = new JsonArray();
        int limit = request.get("listing_limit").getAsInt();
        boolean truncated = false;
        for (var ins : currentProgram.getListing().getInstructions(f.getBody(), true)) {
            if (listing.size() >= limit) { truncated = true; break; }
            var row = new JsonObject();
            row.addProperty("address", hex(ins.getAddress()));
            row.addProperty("text", ins.toString());
            listing.add(row);
        }
        result.add("listing", listing);
        result.addProperty("listing_truncated", truncated);
        var output = decompiler.decompileFunction(f, request.get("decompile_timeout").getAsInt(), monitor);
        result.addProperty("status", output.decompileCompleted() ? "complete" : output.isTimedOut() ? "timeout" : "decompile_failed");
        result.addProperty("diagnostic", output.getErrorMessage());
        if (output.decompileCompleted()) {
            var repairs = new JsonObject();
            result.addProperty("pseudocode", repaired(f, output, repairs));
            result.add("pseudocode_repairs", repairs);
            result.getAsJsonObject("function").addProperty("signature", output.getDecompiledFunction().getSignature());
        }
        return result;
    }
    /**
     * Pseudocode with two artefacts of Ghidra's 68000 model repaired, counted in
     * `repairs`. (1) SLEIGH flag macros read an operand once per flag
     * (`tst.l x` sets N and Z from two loads), and volatile memory keeps both
     * reads, so one instruction prints `uVar1 = x; iVar2 = x;`: the read whose
     * result is unused is dropped. (2) A constant from register-relative
     * addressing (`lea -1254(a5),a0`) used in pointer arithmetic prints as a
     * bare integer: it is named `&symbol` when this function references that
     * address from a register-relative operand.
     */
    private String repaired(Function f, ghidra.app.decompiler.DecompileResults output, JsonObject repairs) {
        var original = output.getDecompiledFunction().getC();
        repairs.addProperty("volatile_duplicate_reads_dropped", 0);
        repairs.addProperty("register_relative_addresses_named", 0);
        var markup = output.getCCodeMarkup();
        var lines = new ghidra.app.decompiler.PrettyPrinter(f, markup, null).getLines();
        var memory = currentProgram.getMemory();
        // Volatile reads per (instruction, address); drop dead ones while another remains.
        var reads = new java.util.HashMap<String, java.util.List<ghidra.program.model.pcode.PcodeOp>>();
        var statements = new ArrayList<ghidra.app.decompiler.ClangStatement>();
        collectStatements(markup, statements);
        for (var st : statements) {
            var op = st.getPcodeOp();
            if (op == null || op.getOpcode() != ghidra.program.model.pcode.PcodeOp.CALLOTHER || op.getOutput() == null || op.getNumInputs() != 2) continue;
            var src = op.getInput(1);
            if (!src.isAddress()) continue;
            var block = memory.getBlock(src.getAddress());
            if (block == null || !block.isVolatile()) continue;
            reads.computeIfAbsent(op.getSeqnum().getTarget() + "/" + src.getAddress(), k -> new ArrayList<>()).add(op);
        }
        var droppedOps = new java.util.HashSet<ghidra.program.model.pcode.PcodeOp>();
        var droppedVars = new java.util.HashSet<ghidra.program.model.pcode.HighVariable>();
        for (var group : reads.values()) {
            if (group.size() < 2) continue;
            int kept = group.size();
            for (var op : group) {
                if (kept == 1) break;
                if (op.getOutput().getDescendants().hasNext()) continue;
                droppedOps.add(op);
                if (op.getOutput().getHigh() != null) droppedVars.add(op.getOutput().getHigh());
                kept--;
            }
        }
        // Register-relative reference targets of this function.
        var relative = new java.util.HashSet<Long>();
        for (var ins : currentProgram.getListing().getInstructions(f.getBody(), true)) {
            for (var ref : ins.getReferencesFrom()) {
                if (!ref.getReferenceType().isData() || !ref.getToAddress().isMemoryAddress() || ref.getOperandIndex() < 0) continue;
                for (var o : ins.getOpObjects(ref.getOperandIndex())) {
                    if (o instanceof ghidra.program.model.lang.Register) { relative.add(ref.getToAddress().getOffset()); break; }
                }
            }
        }
        var text = new java.util.HashMap<ghidra.app.decompiler.ClangToken, String>();
        int named = 0;
        for (var line : lines) {
            var tokens = line.getAllTokens();
            for (int i = 0; i < tokens.size(); i++) {
                var t = tokens.get(i);
                if (!(t instanceof ghidra.app.decompiler.ClangVariableToken)) continue;
                var vn = t.getVarnode();
                var op = t.getPcodeOp();
                if (vn == null || !vn.isConstant() || vn.getSize() != 4 || op == null) continue;
                int code = op.getOpcode();
                if (code != ghidra.program.model.pcode.PcodeOp.INT_ADD && code != ghidra.program.model.pcode.PcodeOp.INT_SUB) continue;
                if (!relative.contains(vn.getOffset())) continue;
                var sym = getSymbolAt(toAddr(vn.getOffset()));
                if (sym == null) continue;
                var high = vn.getHigh();
                boolean pointer = high != null && high.getDataType() instanceof ghidra.program.model.data.Pointer;
                text.put(t, (pointer ? "&" : "(int)&") + sym.getName());
                // An added 0xFFFFFE2C may print as `x - 0x1d4`; the name is added.
                if (code == ghidra.program.model.pcode.PcodeOp.INT_ADD) {
                    for (int j = i - 1; j >= 0; j--) {
                        var prev = tokens.get(j);
                        if (prev.getText().isBlank()) continue;
                        if (prev.getText().equals("-")) text.put(prev, "+");
                        break;
                    }
                }
                named++;
            }
        }
        if (droppedOps.isEmpty() && named == 0) return original;
        // Re-rendering must reproduce Ghidra's text before it is trusted to edit it.
        if (!render(lines, java.util.Set.of(), java.util.Set.of(), java.util.Map.of()).equals(original)) {
            repairs.addProperty("skipped", "token rendering differs from Ghidra's text");
            return original;
        }
        repairs.addProperty("volatile_duplicate_reads_dropped", droppedOps.size());
        repairs.addProperty("register_relative_addresses_named", named);
        return render(lines, droppedOps, droppedVars, text);
    }
    private String render(java.util.List<ghidra.app.decompiler.ClangLine> lines, java.util.Set<ghidra.program.model.pcode.PcodeOp> ops,
            java.util.Set<ghidra.program.model.pcode.HighVariable> vars, java.util.Map<ghidra.app.decompiler.ClangToken, String> text) {
        var out = new StringBuilder();
        for (var line : lines) {
            if (dropLine(line, ops, vars)) continue;
            out.append(line.getIndentString());
            for (var t : line.getAllTokens()) out.append(text.getOrDefault(t, t.getText()));
            out.append('\n');
        }
        return out.toString();
    }
    private void collectStatements(ghidra.app.decompiler.ClangNode n, java.util.List<ghidra.app.decompiler.ClangStatement> out) {
        if (n instanceof ghidra.app.decompiler.ClangStatement st) { out.add(st); return; }
        for (int i = 0; i < n.numChildren(); i++) collectStatements(n.Child(i), out);
    }
    /** A line holding only dropped statements, or only the declaration of a variable they alone used. */
    private boolean dropLine(ghidra.app.decompiler.ClangLine line, java.util.Set<ghidra.program.model.pcode.PcodeOp> ops,
            java.util.Set<ghidra.program.model.pcode.HighVariable> vars) {
        if (ops.isEmpty()) return false;
        boolean any = false;
        for (var t : line.getAllTokens()) {
            ghidra.app.decompiler.ClangNode p = t.Parent();
            while (p != null && !(p instanceof ghidra.app.decompiler.ClangStatement) && !(p instanceof ghidra.app.decompiler.ClangVariableDecl)) {
                p = p.Parent();
            }
            if (p instanceof ghidra.app.decompiler.ClangStatement st && ops.contains(st.getPcodeOp())) { any = true; continue; }
            if (p instanceof ghidra.app.decompiler.ClangVariableDecl d && vars.contains(d.getHighVariable()) && !usedElsewhere(d.getHighVariable(), ops)) { any = true; continue; }
            // The terminating `;` belongs to the enclosing block, not the statement.
            if (!t.getText().isBlank() && !t.getText().equals(";")) return false;
        }
        return any;
    }
    /** True if the variable is read or written by an op that stays in the output. */
    private boolean usedElsewhere(ghidra.program.model.pcode.HighVariable h, java.util.Set<ghidra.program.model.pcode.PcodeOp> dropped) {
        for (var vn : h.getInstances()) {
            var def = vn.getDef();
            if (def != null && !dropped.contains(def)) return true;
            for (var it = vn.getDescendants(); it.hasNext();) if (!dropped.contains(it.next())) return true;
        }
        return false;
    }
    @Override public void run() throws Exception {
        try (var in = Files.newBufferedReader(Path.of(getScriptArgs()[0]), StandardCharsets.UTF_8)) {
            request = JsonParser.parseReader(in).getAsJsonObject();
        }
        String mode = request.get("mode").getAsString();
        if (mode.equals("setup")) { setup(); return; }
        if (mode.equals("annotate")) { annotate(); return; }
        if (mode.equals("export")) exportIndex();
        if (request.has("addrs")) decompile();
    }
}
