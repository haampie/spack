"""Profile Python bytecode execution to identify hot paths and suggest code optimizations.

This profiler uses sys.monitoring to track branch execution, builds a control flow graph,
and identifies hot paths through the code. It can suggest reordering conditions to reduce
jumps and improve CPU branch prediction.
"""

import dis
import inspect
import runpy
import sys
import types
from typing import Dict, List, Optional, Tuple

# ============================================================================
# Profiling with sys.monitoring
# ============================================================================

mon = sys.monitoring
TOOL_ID = mon.PROFILER_ID

# Track how many times each edge (code_object, from_offset, to_offset) is taken
edge_counts: Dict[Tuple[types.CodeType, int, int], int] = {}

# Track total bytes jumped across all executions
total_bytes_jumped: int = 0


def branch_handler(code: types.CodeType, from_offset: int, to_offset: int):
    """Called by sys.monitoring for each branch/jump taken during execution."""
    global total_bytes_jumped
    key = (code, from_offset, to_offset)
    edge_counts[key] = edge_counts.get(key, 0) + 1
    
    # Track bytes jumped
    if to_offset > from_offset:
        total_bytes_jumped += to_offset - from_offset


def start():
    """Start profiling by registering monitoring callbacks."""
    mon.use_tool_id(TOOL_ID, "EdgeProfiler")
    mon.register_callback(TOOL_ID, mon.events.BRANCH_LEFT, branch_handler)
    mon.register_callback(TOOL_ID, mon.events.BRANCH_RIGHT, branch_handler)
    mon.register_callback(TOOL_ID, mon.events.JUMP, branch_handler)

    events = mon.events.BRANCH_LEFT | mon.events.BRANCH_RIGHT | mon.events.JUMP
    mon.set_events(TOOL_ID, events)


def stop():
    """Stop profiling and unregister callbacks."""
    mon.set_events(TOOL_ID, 0)
    mon.free_tool_id(TOOL_ID)


# ============================================================================
# Control Flow Graph (CFG) Construction
# ============================================================================


class BasicBlock:
    """Represents a basic block in the control flow graph.

    A basic block is a sequence of instructions with:
    - One entry point (the first instruction)
    - One exit point (the last instruction)
    - No branches in or out except at entry and exit
    """

    def __init__(self, start_offset: int):
        self.start_offset = start_offset
        self.end_offset = -1
        self.instructions: List[dis.Instruction] = []
        self.successors: List[BasicBlock] = []
        self.predecessors: List[BasicBlock] = []
        self.edges: Dict[BasicBlock, int] = {}  # BasicBlock -> execution count (int)

    def __repr__(self):
        return f"<BasicBlock {self.start_offset}>"


class ControlFlowGraph:
    """Constructs and analyzes a control flow graph from bytecode.

    The CFG is built by:
    1. Identifying basic block leaders (entry points)
    2. Creating basic blocks between leaders
    3. Connecting blocks based on jumps and fallthroughs
    4. Applying execution weights from profiling data
    """

    def __init__(self, code: types.CodeType):
        self.code = code
        self.blocks = {}  # start_offset -> BasicBlock
        self.build()

    def _get_branch_map(self) -> Dict[int, Tuple[int, int]]:
        """Extract branch targets from code object's co_branches()."""
        branch_map = {}
        try:
            for start, t1, t2 in self.code.co_branches():
                branch_map[start] = (t1, t2)
        except AttributeError:
            pass
        return branch_map

    def build(self):
        """Build the control flow graph by identifying blocks and connecting them."""
        branch_map = self._get_branch_map()

        # Step 1: Identify basic block leaders (entry points)
        # - Offset 0 (start of function) is always a leader
        # - Jump targets are leaders
        # - Instructions following unconditional jumps/returns are leaders
        leaders = {0}
        instructions = list(dis.get_instructions(self.code))
        offset_to_idx = {instr.offset: i for i, instr in enumerate(instructions)}

        # Add branch targets as leaders
        for t1, t2 in branch_map.values():
            leaders.add(t1)
            leaders.add(t2)

        # Scan instructions to find additional leaders
        for i, instr in enumerate(instructions):
            if instr.is_jump_target:
                leaders.add(instr.offset)

            # Instruction following control flow changes are leaders
            if instr.opname in ("JUMP_FORWARD", "JUMP_BACKWARD", "JUMP_BACKWARD_NO_INTERRUPT"):
                if instr.argval is not None:
                    leaders.add(instr.argval)
                if i + 1 < len(instructions):
                    leaders.add(instructions[i + 1].offset)
            elif instr.opname in ("RETURN_VALUE", "RETURN_CONST", "RAISE_VARARGS", "RERAISE"):
                if i + 1 < len(instructions):
                    leaders.add(instructions[i + 1].offset)

        # Step 2: Create basic blocks from leaders
        sorted_leaders = sorted(leaders)
        for start_offset in sorted_leaders:
            block = BasicBlock(start_offset)
            self.blocks[start_offset] = block

            # Collect instructions until we hit the next leader
            if start_offset in offset_to_idx:
                instr_idx = offset_to_idx[start_offset]
                while instr_idx < len(instructions):
                    instr = instructions[instr_idx]
                    if instr.offset in leaders and instr.offset != start_offset:
                        break
                    block.instructions.append(instr)
                    instr_idx += 1

                # Set end offset
                if block.instructions:
                    if instr_idx < len(instructions):
                        block.end_offset = instructions[instr_idx].offset
                    else:
                        # End of code - approximate
                        block.end_offset = block.instructions[-1].offset + 2

        # Step 3: Connect blocks with edges
        for block in self.blocks.values():
            if not block.instructions:
                continue

            # Find branch instruction (may be followed by CACHE instructions)
            branch_instr = self._find_branch_instruction(block, branch_map)

            if branch_instr:
                # Conditional branch: connect to both targets
                t1, t2 = branch_map[branch_instr.offset]
                if t1 in self.blocks:
                    self.add_edge(block, self.blocks[t1])
                if t2 in self.blocks:
                    self.add_edge(block, self.blocks[t2])
            else:
                last_instr = block.instructions[-1]

                # Unconditional jump
                if last_instr.opname in (
                    "JUMP_FORWARD",
                    "JUMP_BACKWARD",
                    "JUMP_BACKWARD_NO_INTERRUPT",
                ):
                    target = last_instr.argval
                    if target in self.blocks:
                        self.add_edge(block, self.blocks[target])
                # Return/raise: no successors
                elif last_instr.opname in (
                    "RETURN_VALUE",
                    "RETURN_CONST",
                    "RAISE_VARARGS",
                    "RERAISE",
                ):
                    pass
                # Fallthrough to next block
                else:
                    next_block = self.get_next_block(block)
                    if next_block:
                        self.add_edge(block, next_block)

    def _find_branch_instruction(self, block: BasicBlock, branch_map: Dict[int, Tuple[int, int]]):
        """Find the branch instruction in a block, if any."""
        for instr in reversed(block.instructions):
            if instr.offset in branch_map:
                return instr
        return None

    def get_next_block(self, block: BasicBlock) -> Optional[BasicBlock]:
        """Get the next sequential block after the given block."""
        sorted_keys = sorted(self.blocks.keys())
        try:
            idx = sorted_keys.index(block.start_offset)
            if idx + 1 < len(sorted_keys):
                return self.blocks[sorted_keys[idx + 1]]
        except ValueError:
            pass
        return None

    def add_edge(self, source: BasicBlock, dest: BasicBlock):
        """Add a directed edge from source to dest block."""
        if dest not in source.successors:
            source.successors.append(dest)
        if source not in dest.predecessors:
            dest.predecessors.append(source)
        if dest not in source.edges:
            source.edges[dest] = 0  # Initialize weight

    def apply_weights(self, counts: Dict[Tuple[types.CodeType, int, int], int]):
        """Apply execution counts to CFG edges from profiling data."""
        branch_map = self._get_branch_map()

        for block in self.blocks.values():
            if not block.instructions:
                continue

            branch_instr = self._find_branch_instruction(block, branch_map)

            if branch_instr:
                # Conditional branch: apply weights to both targets
                t1, t2 = branch_map[branch_instr.offset]
                self._set_edge_weight(
                    block, t1, counts.get((self.code, branch_instr.offset, t1), 0)
                )
                self._set_edge_weight(
                    block, t2, counts.get((self.code, branch_instr.offset, t2), 0)
                )
            else:
                # Unconditional jump
                last_instr = block.instructions[-1]
                if last_instr.opname in (
                    "JUMP_FORWARD",
                    "JUMP_BACKWARD",
                    "JUMP_BACKWARD_NO_INTERRUPT",
                ):
                    target = last_instr.argval
                    weight = counts.get((self.code, last_instr.offset, target), 0)
                    self._set_edge_weight(block, target, weight)

    def _set_edge_weight(self, block: BasicBlock, target_offset: int, weight: int):
        """Set the weight for an edge from block to target."""
        if target_offset in self.blocks:
            target_block = self.blocks[target_offset]
            if target_block in block.edges:
                block.edges[target_block] = weight

    def form_chains(self) -> List[List[BasicBlock]]:
        """Form chains of basic blocks by greedily connecting hot edges.

        This implements a greedy algorithm:
        1. Sort edges by execution count (weight)
        2. Merge chains when the hottest edge connects a tail to a head
        3. Return the resulting chains
        """
        # Collect weighted edges
        edges = []
        for source in self.blocks.values():
            for dest, weight in source.edges.items():
                if weight > 0:
                    edges.append((source, dest, weight))

        edges.sort(key=lambda x: x[2], reverse=True)

        # Each block starts as its own chain
        block_to_chain = {b: [b] for b in self.blocks.values()}

        # Greedily merge chains along hot edges
        for source, dest, weight in edges:
            chain_source = block_to_chain[source]
            chain_dest = block_to_chain[dest]

            # Can only merge if source is tail and dest is head
            if chain_source is not chain_dest:
                if chain_source[-1] is source and chain_dest[0] is dest:
                    new_chain = chain_source + chain_dest
                    for b in new_chain:
                        block_to_chain[b] = new_chain

        # Extract unique chains (deterministic order by start offset)
        unique_chains = []
        seen_ids = set()
        for b in sorted(block_to_chain.keys(), key=lambda b: b.start_offset):
            chain = block_to_chain[b]
            if id(chain) not in seen_ids:
                unique_chains.append(chain)
                seen_ids.add(id(chain))

        return unique_chains

    def get_source_context(self) -> Tuple[Optional[List[str]], Optional[int]]:
        """Retrieve the source code lines for this code object."""
        try:
            lines, start_lineno = inspect.getsourcelines(self.code)
            return lines, start_lineno
        except OSError:
            return None, None

    def print_chains(self, chains: List[List[BasicBlock]]):
        """Print the chains with source code context and execution weights."""
        lines, start_lineno = self.get_source_context()
        print("  Generated Chains:")

        for i, chain in enumerate(chains):
            print(f"    Chain {i + 1}:")
            for j, block in enumerate(chain):
                # Get edge weight to next block
                weight_str = ""
                if j < len(chain) - 1:
                    weight_str = self._format_edge_weight(block, chain[j + 1])

                # Get line range for this block
                positions = self._get_block_positions(block)

                if not positions or not lines or start_lineno is None:
                    print(f"      Block {block.start_offset}{weight_str}")
                else:
                    self._print_block_with_source(
                        block, positions, lines, start_lineno, weight_str
                    )

    def _get_block_positions(self, block: BasicBlock) -> List:
        """Extract position information from block instructions."""
        positions = []
        for instr in block.instructions:
            if hasattr(instr, "positions") and instr.positions:
                if instr.positions.lineno is not None:
                    positions.append(instr.positions)
        return positions

    def _format_edge_weight(self, source: BasicBlock, dest: BasicBlock) -> str:
        """Format the edge weight and jump distance information."""
        weight = source.edges.get(dest, 0)
        distance = dest.start_offset - source.end_offset

        jump_info = " (contiguous)" if distance == 0 else f" (jump: {distance:+d} bytes)"
        return f" --(weight: {weight}{jump_info})--> "

    def _print_block_with_source(
        self,
        block: BasicBlock,
        positions: List,
        lines: List[str],
        start_lineno: int,
        weight_str: str,
    ):
        """Print a block with its source code lines."""
        try:
            min_lineno = min(p.lineno for p in positions)
            max_lineno = max(p.lineno for p in positions)

            print(f"      Block {block.start_offset} (lines {min_lineno}-{max_lineno}){weight_str}")

            # Print source lines
            for lineno in range(min_lineno, max_lineno + 1):
                line_idx = lineno - start_lineno
                if 0 <= line_idx < len(lines):
                    line_content = lines[line_idx].rstrip()
                    print(f"        {lineno}: {line_content}")

                    # Highlight single-line blocks
                    if min_lineno == max_lineno:
                        self._print_line_highlight(lineno, positions, line_content)
        except ValueError:
            print(f"      Block {block.start_offset}{weight_str}")

    def _print_line_highlight(self, lineno: int, positions: List, line_content: str):
        """Print column highlighting for single-line blocks."""
        cols = [
            (p.col_offset, p.end_col_offset)
            for p in positions
            if p.lineno == lineno and p.col_offset is not None
        ]
        if cols:
            min_col = min(c[0] for c in cols)
            max_col = max(c[1] for c in cols)

            if min_col < len(line_content):
                prefix_len = 8 + len(str(lineno)) + 2
                underline = " " * (prefix_len + min_col) + "^" * (max_col - min_col)
                print(underline)

    def collect_advice(self, chains: List[List[BasicBlock]]) -> List[str]:
        """Collect optimization advice for hot paths that require jumps."""
        advice_list = []
        
        lines, start_lineno = self.get_source_context()
        if not lines or start_lineno is None:
            return advice_list

        for chain in chains:
            for i in range(len(chain) - 1):
                advice = self._analyze_edge_for_advice(chain[i], chain[i + 1], lines, start_lineno)
                if advice:
                    advice_list.append(advice)
        
        return advice_list

    def generate_advice(self, chains: List[List[BasicBlock]]):
        """Generate and print optimization advice for hot paths that require jumps."""
        advice_list = self.collect_advice(chains)
        
        if advice_list:
            print("  Optimization Advice:")
            for advice in advice_list:
                # Indent each line of advice
                for line in advice.split('\n'):
                    print(f"    {line}")

    def _analyze_edge_for_advice(
        self, u: BasicBlock, v: BasicBlock, lines: List[str], start_lineno: int
    ) -> Optional[str]:
        """Analyze an edge and generate advice if it requires an unnecessary jump."""
        if not u.instructions:
            return None

        # Only analyze non-contiguous edges (jumps)
        distance = v.start_offset - u.end_offset
        if distance <= 0:
            return None

        # Find conditional jump instruction
        jump_instr = self._find_conditional_jump(u)
        if not jump_instr:
            return None

        target = jump_instr.argval

        # Only advise if we're jumping to the hot path (should be fallthrough instead)
        if v.start_offset != target:
            return None

        # Calculate execution probability
        total_weight = sum(u.edges.values())
        taken_weight = u.edges.get(v, 0)

        if total_weight == 0:
            return None

        taken_prob = taken_weight / total_weight

        # Get source location
        lineno, condition_text = self._get_condition_info(u, jump_instr, lines, start_lineno)

        if not lineno:
            return None

        return (
            f"Line {lineno}: '{condition_text}'\n"
            f"  -> Hot path jumps {distance:+d} bytes (taken {taken_prob:.1%} of the time)."
        )

    def _find_conditional_jump(self, block: BasicBlock):
        """Find the conditional jump instruction in a block."""
        for instr in reversed(block.instructions):
            if "JUMP_IF" in instr.opname:
                return instr
        return None

    def _get_condition_info(
        self, block: BasicBlock, jump_instr, lines: List[str], start_lineno: int
    ) -> Tuple[Optional[int], str]:
        """Extract line number and condition text for a jump instruction."""
        # Try to find the instruction that evaluates the condition
        condition_instr = None
        idx = block.instructions.index(jump_instr)
        if idx > 0:
            for k in range(idx - 1, -1, -1):
                prev = block.instructions[k]
                if prev.opname not in ("CACHE", "EXTENDED_ARG", "KW_NAMES"):
                    condition_instr = prev
                    break

        target_instr = condition_instr if condition_instr else jump_instr

        # Extract position information
        lineno = None
        col_offset = None
        end_col_offset = None

        if hasattr(target_instr, "positions") and target_instr.positions:
            lineno = target_instr.positions.lineno
            col_offset = target_instr.positions.col_offset
            end_col_offset = target_instr.positions.end_col_offset
        elif hasattr(target_instr, "starts_line") and target_instr.starts_line is not None:
            lineno = target_instr.starts_line

        if not lineno:
            return None, ""

        # Extract condition text
        try:
            line_idx = lineno - start_lineno
            if 0 <= line_idx < len(lines):
                src_line = lines[line_idx].rstrip()

                if (
                    col_offset is not None
                    and end_col_offset is not None
                    and end_col_offset <= len(src_line)
                ):
                    condition_text = src_line[col_offset:end_col_offset]
                else:
                    condition_text = src_line.strip()

                return lineno, condition_text
        except IndexError:
            pass

        return None, ""

    def print_graph(self):
        """Print the raw control flow graph structure."""
        for start_offset in sorted(self.blocks.keys()):
            block = self.blocks[start_offset]
            print(f"Block {start_offset}:")
            for instr in block.instructions:
                argval = instr.argval if instr.argval is not None else ""
                print(f"  {instr.offset}: {instr.opname} {argval}")

            print("  Successors:")
            for succ in block.successors:
                weight = block.edges.get(succ, 0)
                print(f"    -> Block {succ.start_offset} (weight: {weight})")
            print("-" * 20)


# ============================================================================
# Reporting
# ============================================================================


def report():
    """Generate a report of the most costly chains across all profiled code."""
    all_chains_info = []
    
    # Identify all code objects
    code_objects = set(k[0] for k in edge_counts.keys())
    
    for code in code_objects:
        try:
            cfg = ControlFlowGraph(code)
            cfg.apply_weights(edge_counts)
            chains = cfg.form_chains()
            
            for chain in chains:
                chain_cost = 0
                for i in range(len(chain) - 1):
                    u, v = chain[i], chain[i + 1]
                    distance = v.start_offset - u.end_offset
                    
                    # Only count forward jumps that are conditional
                    if distance > 0:
                        jump_instr = cfg._find_conditional_jump(u)
                        if jump_instr and jump_instr.argval == v.start_offset:
                            weight = u.edges.get(v, 0)
                            chain_cost += weight * distance
                
                if chain_cost > 0:
                    all_chains_info.append({
                        'cost': chain_cost,
                        'chain': chain,
                        'cfg': cfg,
                        'code': code
                    })
        except Exception:
            continue

    # Sort by cost
    all_chains_info.sort(key=lambda x: x['cost'], reverse=True)

    print(f"\n{'=' * 60}")
    print(f"Top 10 Most Costly Chains")
    print(f"{'=' * 60}")

    for i, info in enumerate(all_chains_info[:10]):
        code = info['code']
        cost = info['cost']
        chain = info['chain']
        cfg = info['cfg']
        
        print(f"\n#{i+1} Cost: {cost:,} bytes jumped")
        print(f"In {code.co_name} ({code.co_filename}:{code.co_firstlineno})")
        
        # Print the chain
        cfg.print_chains([chain])
        
        # Print advice for this chain
        cfg.generate_advice([chain])

    # Report total bytes jumped
    print(f"\n{'=' * 60}")
    print(f"Total bytes jumped forward: {total_bytes_jumped:,}")
    print(f"{'=' * 60}")


# ============================================================================
# Main entry point
# ============================================================================

if __name__ == "__main__":

    if len(sys.argv) < 2:
        print("Usage: python -m profiler <script.py> [args...]", file=sys.stderr)
        sys.exit(1)

    script_path = sys.argv[1]
    
    sys.argv = sys.argv[1:]
    
    start()
    try:
        runpy.run_path(script_path, run_name="__main__")
    finally:
        stop()
        report()
