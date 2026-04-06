"""
lldb_helpers.py — Breakpoint handler functions for mini-agent-loop debugging.

Loaded by lldbinit via: command script import debug/lldb_helpers.py

Key design: Read Rust String/&str values directly from process memory,
because LLDB's GetSummary()/GetValue() often fails for Rust types.
We also leverage Rust pretty printers loaded by lldbinit.
"""
import lldb


# ============================================================================
# Rust type readers — extract values from Rust types in LLDB
# ============================================================================

def read_rust_string(var):
    """Read a Rust `String` value from memory.

    Rust String layout: String { vec: Vec<u8> { buf: RawVec { inner: ... { ptr: { pointer } } }, len } }
    """
    if not var or not var.IsValid():
        return None

    # First try GetSummary() — works if Rust pretty printers are loaded
    s = var.GetSummary()
    if s:
        return s.strip('"')

    # Manual memory read: String -> vec -> buf -> inner -> ptr -> pointer, and vec -> len
    try:
        raw = var.GetNonSyntheticValue()
        vec = raw.GetChildMemberWithName("vec")
        if not vec.IsValid():
            vec = raw  # might already be the vec

        vec_raw = vec.GetNonSyntheticValue()

        # Get pointer
        pointer = (
            vec_raw.GetChildMemberWithName("buf")
            .GetChildMemberWithName("inner")
            .GetChildMemberWithName("ptr")
            .GetChildMemberWithName("pointer")
            .GetChildMemberWithName("pointer")
        )

        # Get length
        length = vec_raw.GetChildMemberWithName("len").GetValueAsUnsigned()

        if length == 0:
            return ""
        if length > 10000:
            length = 10000  # safety cap

        addr = pointer.GetValueAsUnsigned()
        if addr == 0:
            return None

        error = lldb.SBError()
        process = pointer.GetProcess()
        data = process.ReadMemory(addr, length, error)
        if error.Success() and data:
            return data.decode("utf-8", "replace")
    except Exception:
        pass

    return None


def read_rust_str_ref(var):
    """Read a Rust `&str` value from memory.

    &str layout: { data_ptr: *const u8, length: usize }
    """
    if not var or not var.IsValid():
        return None

    s = var.GetSummary()
    if s:
        return s.strip('"')

    try:
        raw = var.GetNonSyntheticValue()
        data_ptr = raw.GetChildMemberWithName("data_ptr")
        length = raw.GetChildMemberWithName("length").GetValueAsUnsigned()

        if length == 0:
            return ""
        if length > 10000:
            length = 10000

        addr = data_ptr.GetValueAsUnsigned()
        if addr == 0:
            return None

        error = lldb.SBError()
        process = data_ptr.GetProcess()
        data = process.ReadMemory(addr, length, error)
        if error.Success() and data:
            return data.decode("utf-8", "replace")
    except Exception:
        pass

    return None


def read_rust_option_string(var):
    """Read a Rust `Option<String>` value.

    Returns the string if Some, "None" if None, or a fallback.
    """
    if not var or not var.IsValid():
        return None

    s = var.GetSummary()
    if s:
        return s.strip('"')

    type_name = var.GetTypeName() or ""

    # Check discriminant — Option<String> is an enum
    # In LLDB, Option::None often has 0 children or a specific discriminant
    num_children = var.GetNumChildren()

    # Try to find the inner value
    for i in range(num_children):
        child = var.GetChildAtIndex(i)
        child_name = child.GetName() or ""
        # Some variant usually has a child named "__0" or "0"
        if child_name in ("__0", "0"):
            result = read_rust_string(child)
            if result is not None:
                return result

    # Try direct summary
    if "None" in type_name or num_children == 0:
        return "None"

    return None


def get_var(frame, name):
    """Get a variable's string value, trying multiple strategies."""
    var = frame.FindVariable(name)
    if not var.IsValid():
        return "<not found: %s>" % name

    type_name = var.GetTypeName() or ""

    # Try Rust String
    if "String" in type_name and "Option" not in type_name:
        result = read_rust_string(var)
        if result is not None:
            return '"%s"' % result

    # Try &str
    if "str" in type_name and "String" not in type_name:
        result = read_rust_str_ref(var)
        if result is not None:
            return '"%s"' % result

    # Try Option<String>
    if "Option" in type_name:
        result = read_rust_option_string(var)
        if result is not None:
            return result

    # Fallback: GetSummary or GetValue
    s = var.GetSummary()
    if s:
        return s
    v = var.GetValue()
    if v:
        return v

    return str(var)


def get_field(var, field_name):
    """Get a struct field's string value."""
    child = var.GetChildMemberWithName(field_name)
    if not child.IsValid():
        return "<no field: %s>" % field_name

    type_name = child.GetTypeName() or ""

    if "String" in type_name and "Option" not in type_name:
        result = read_rust_string(child)
        if result is not None:
            return '"%s"' % result

    if "str" in type_name and "String" not in type_name:
        result = read_rust_str_ref(child)
        if result is not None:
            return '"%s"' % result

    if "Option" in type_name:
        result = read_rust_option_string(child)
        if result is not None:
            return result

    s = child.GetSummary()
    if s:
        return s
    v = child.GetValue()
    if v:
        return v

    return str(child)


def trunc(s, maxlen=200):
    """Truncate a string for display."""
    if s and len(s) > maxlen:
        return s[:maxlen - 3] + "..."
    return s


def sep(char="="):
    return char * 60


def header(bp_name, location):
    """Print a breakpoint header."""
    print("\n" + sep())
    print("[%s]  %s" % (bp_name, location))
    print(sep("-"))


def footer():
    print(sep())


def print_messages(frame, var_name="*messages"):
    """Print a summary of the messages Vec<ChatMessage>."""
    msgs = frame.FindVariable(var_name)
    if not msgs.IsValid():
        msgs = frame.FindVariable("messages")
    if not msgs.IsValid():
        print("  messages: <not available>")
        return

    n = msgs.GetNumChildren()
    print("  messages: %d message(s)" % n)
    for i in range(n):
        msg = msgs.GetChildAtIndex(i)
        role = get_field(msg, "role")
        content_raw = get_field(msg, "content")
        content = trunc(content_raw, 120)
        tool_call_id = get_field(msg, "tool_call_id")

        line = "    [%d] role=%-10s" % (i, role)
        if tool_call_id and tool_call_id != "None" and "no field" not in tool_call_id:
            line += "  tool_call_id=%s" % tool_call_id
        line += "  content=%s" % content
        print(line)


def print_tool_calls(frame, var_name="tool_calls"):
    """Print tool calls from a Vec<ToolCall>."""
    tcs = frame.FindVariable(var_name)
    if not tcs.IsValid():
        print("  tool_calls: <not available>")
        return

    n = tcs.GetNumChildren()
    print("  tool_calls: %d call(s)" % n)
    for i in range(n):
        tc = tcs.GetChildAtIndex(i)
        tc_id = get_field(tc, "id")
        tc_name = get_field(tc, "name")
        tc_args = trunc(get_field(tc, "arguments"), 150)
        print("    [%d] id=%s  name=%s" % (i, tc_id, tc_name))
        print("         args=%s" % tc_args)


def print_var(frame, name, maxlen=200):
    """Print a single variable with truncation."""
    val = get_var(frame, name)
    print("  %s = %s" % (name, trunc(val, maxlen)))


# ============================================================================
# Breakpoint handlers — each returns False to auto-continue
# ============================================================================

def bp1_user_input(frame, bp_loc, internal_dict):
    header("BP1", "USER INPUT  (main.rs:164)")
    print_var(frame, "input", 500)
    footer()
    return False


def bp2_build_messages(frame, bp_loc, internal_dict):
    header("BP2", "BUILD MESSAGES  (main.rs:189)")
    print_var(frame, "current_model")
    footer()
    return False


def bp3_enter_loop(frame, bp_loc, internal_dict):
    header("BP3", "ENTER AGENTIC LOOP  (main.rs:192)")
    print_messages(frame, "messages")
    footer()
    return False


def bp4_loop_entry(frame, bp_loc, internal_dict):
    header("BP4", "AGENTIC LOOP ENTRY  (agent.rs:68)")
    # config is a reference, dereference it
    cfg = frame.FindVariable("config")
    if cfg.IsValid():
        deref = cfg.Dereference()
        if deref.IsValid():
            mi = get_field(deref, "max_iterations")
            print("  config.max_iterations = %s" % mi)
    print_messages(frame)
    footer()
    return False


def bp5_before_llm(frame, bp_loc, internal_dict):
    header("BP5", "BEFORE LLM CALL  (agent.rs:75)")
    print_var(frame, "iteration")
    # Count messages
    msgs = frame.FindVariable("*messages")
    if not msgs.IsValid():
        msgs = frame.FindVariable("messages")
    if msgs.IsValid():
        print("  messages: %d message(s)" % msgs.GetNumChildren())
    # Count tool defs
    td = frame.FindVariable("tool_defs")
    if td.IsValid():
        print("  tool_defs: %d tool(s)" % td.GetNumChildren())
    footer()
    return False


def bp6_llm_response(frame, bp_loc, internal_dict):
    header("BP6", "LLM RESPONSE RECEIVED  (agent.rs:77)")
    resp = frame.FindVariable("response")
    if resp.IsValid():
        # Try to get finish_reason
        fr = resp.GetChildMemberWithName("finish_reason")
        if fr.IsValid():
            fr_val = fr.GetValue() or fr.GetSummary() or str(fr)
            print("  finish_reason = %s" % fr_val)

        # Try to get result type info
        result = resp.GetChildMemberWithName("result")
        if result.IsValid():
            type_name = result.GetTypeName() or ""
            print("  result type = %s" % type_name)
            # Try to get summary
            s = result.GetSummary()
            if s:
                print("  result = %s" % trunc(s, 300))
    else:
        print("  response: <not available>")
    footer()
    return False


def bp7_text_response(frame, bp_loc, internal_dict):
    header("BP7", "LLM TEXT RESPONSE  (agent.rs:80)")
    print_var(frame, "text", 500)
    footer()
    return False


def bp8_tool_calls(frame, bp_loc, internal_dict):
    header("BP8", "LLM TOOL CALLS  (agent.rs:89)")
    print_var(frame, "content")
    print_tool_calls(frame)
    footer()
    return False


def bp9_tool_exec(frame, bp_loc, internal_dict):
    header("BP9", "TOOL EXECUTION  (execute_tool_with_safety)")
    print_var(frame, "tool_name")
    print_var(frame, "params", 300)
    footer()
    return False


def bp10_tool_result(frame, bp_loc, internal_dict):
    header("BP10", "TOOL RESULT  (process_tool_result)")
    print_var(frame, "tool_name")
    print_var(frame, "tool_call_id")
    print_var(frame, "result", 300)
    footer()
    return False


def bp11_calculator(frame, bp_loc, internal_dict):
    header("BP11", "CALCULATOR TOOL  (calculator.rs:48)")
    print_var(frame, "params", 300)
    footer()
    return False


def bp12_openai_chat(frame, bp_loc, internal_dict):
    header("BP12", "OPENAI CHAT REQUEST  (openai.rs:275)")
    self_var = frame.FindVariable("self")
    if self_var.IsValid():
        deref = self_var.Dereference()
        if deref.IsValid():
            print("  model = %s" % get_field(deref, "model"))
            print("  base_url = %s" % get_field(deref, "base_url"))
    msgs = frame.FindVariable("messages")
    if msgs.IsValid():
        print("  messages: %d message(s)" % msgs.GetNumChildren())
    tools = frame.FindVariable("tools")
    if tools.IsValid():
        print("  tools: %d tool(s)" % tools.GetNumChildren())
    footer()
    return False


def bp13_openai_parse(frame, bp_loc, internal_dict):
    header("BP13", "OPENAI RESPONSE PARSE  (openai.rs:306)")
    print_var(frame, "status")
    footer()
    return False


def bp14_final(frame, bp_loc, internal_dict):
    header("BP14", "FINAL RESPONSE  (main.rs:193)")
    print_var(frame, "text", 800)
    footer()
    return False
