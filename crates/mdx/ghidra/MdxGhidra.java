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
        exportIndex();
    }
    private void decompile() throws Exception {
        var result = new JsonObject();
        var a = toAddr(request.get("addr").getAsLong());
        var f = getFunctionContaining(a);
        if (f == null) {
            result.addProperty("status", "function_not_found");
            result.addProperty("diagnostic", "No analysed function contains this address; seed it with entries, symbols or runtime evidence.");
            write("query_result", result);
            return;
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
        var decompiler = new DecompInterface();
        try {
            if (!decompiler.openProgram(currentProgram)) throw new IOException(decompiler.getLastMessage());
            var output = decompiler.decompileFunction(f, request.get("decompile_timeout").getAsInt(), monitor);
            result.addProperty("status", output.decompileCompleted() ? "complete" : output.isTimedOut() ? "timeout" : "decompile_failed");
            result.addProperty("diagnostic", output.getErrorMessage());
            if (output.decompileCompleted()) {
                result.addProperty("pseudocode", output.getDecompiledFunction().getC());
                result.getAsJsonObject("function").addProperty("signature", output.getDecompiledFunction().getSignature());
            }
        } finally {
            decompiler.dispose();
        }
        write("query_result", result);
    }
    @Override public void run() throws Exception {
        try (var in = Files.newBufferedReader(Path.of(getScriptArgs()[0]), StandardCharsets.UTF_8)) {
            request = JsonParser.parseReader(in).getAsJsonObject();
        }
        String mode = request.get("mode").getAsString();
        if (mode.equals("setup")) { setup(); return; }
        if (mode.equals("annotate")) { annotate(); return; }
        if (mode.equals("export")) exportIndex();
        if (request.has("addr")) decompile();
    }
}
