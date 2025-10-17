#!/usr/bin/env python3
"""
Rust Code Structure Analyzer
Analyser automatiquement la structure des fichiers Rust pour migration
"""

import os
import re
import sys
from pathlib import Path

def analyze_rust_file(file_path):
    """Analyse un fichier Rust et extrait sa structure"""
    try:
        with open(file_path, 'r', encoding='utf-8') as f:
            content = f.read()
    except Exception as e:
        return {"file": file_path, "error": str(e)}
    
    lines = content.split('\n')
    analysis = {
        "file": file_path,
        "lines": len(lines),
        "structs": [],
        "functions": [],
        "enums": [],
        "traits": [],
        "impls": [],
        "mods": [],
        "uses": []
    }
    
    for i, line in enumerate(lines, 1):
        line = line.strip()
        
        # Public structs
        if match := re.match(r'^pub struct\s+(\w+)', line):
            analysis["structs"].append({"name": match.group(1), "line": i})
        
        # Public functions
        if match := re.match(r'^pub fn\s+(\w+)', line):
            analysis["functions"].append({"name": match.group(1), "line": i})
            
        # Public enums
        if match := re.match(r'^pub enum\s+(\w+)', line):
            analysis["enums"].append({"name": match.group(1), "line": i})
            
        # Public traits
        if match := re.match(r'^pub trait\s+(\w+)', line):
            analysis["traits"].append({"name": match.group(1), "line": i})
            
        # Impl blocks
        if match := re.match(r'^impl(?:\s*<[^>]*>)?\s+(.+?)\s*(?:{|$)', line):
            impl_name = match.group(1).strip()
            # Clean up generic params and 'for' clauses
            impl_name = re.sub(r'<[^>]*>', '', impl_name)
            impl_name = re.sub(r'\s+for\s+.*', '', impl_name)
            analysis["impls"].append({"name": impl_name, "line": i})
            
        # Modules
        if match := re.match(r'^pub mod\s+(\w+)', line):
            analysis["mods"].append({"name": match.group(1), "line": i})
            
        # Important uses (external crates)
        if match := re.match(r'^use\s+([^:]+)', line):
            if not match.group(1).startswith('crate::') and not match.group(1).startswith('super::'):
                analysis["uses"].append({"name": match.group(1), "line": i})
    
    return analysis

def print_analysis(analysis):
    """Affiche l'analyse de manière formatée"""
    if "error" in analysis:
        print(f"❌ {analysis['file']}: {analysis['error']}")
        return
    
    print(f"📄 {analysis['file']} ({analysis['lines']} lines)")
    
    if analysis["structs"]:
        print("  🏗️  Public Structs:")
        for item in analysis["structs"]:
            print(f"    - {item['name']} (line {item['line']})")
    
    if analysis["functions"]:
        print("  🔧 Public Functions:")
        for item in analysis["functions"]:
            print(f"    - {item['name']} (line {item['line']})")
    
    if analysis["enums"]:
        print("  📋 Public Enums:")
        for item in analysis["enums"]:
            print(f"    - {item['name']} (line {item['line']})")
    
    if analysis["traits"]:
        print("  🎭 Public Traits:")
        for item in analysis["traits"]:
            print(f"    - {item['name']} (line {item['line']})")
    
    if analysis["impls"]:
        print("  ⚙️  Impl Blocks:")
        for item in analysis["impls"]:
            print(f"    - {item['name']} (line {item['line']})")
    
    if analysis["mods"]:
        print("  📦 Public Modules:")
        for item in analysis["mods"]:
            print(f"    - {item['name']} (line {item['line']})")
    
    print()

def find_rust_files(directory, pattern="*.rs"):
    """Trouve tous les fichiers Rust dans un répertoire"""
    path = Path(directory)
    if not path.exists():
        print(f"❌ Directory not found: {directory}")
        return []
    
    rust_files = list(path.rglob(pattern))
    return [str(f) for f in rust_files]

def main():
    # Fichiers clés à analyser
    key_files = [
        "crates/cli/src/core/safe_write.rs",
        "crates/cli/src/core/file_ops.rs", 
        "crates/cli/src/core/path_security.rs",
        "crates/cli/src/core/config.rs",
        "crates/cli/src/core/errors.rs",
        "crates/cli/src/core/journal.rs",
        "crates/cli/src/core/policy.rs"
    ]
    
    print("🔍 Rust Code Structure Analyzer")
    print("=" * 50)
    
    # Analyse des fichiers clés
    print("\n📋 KEY FILES ANALYSIS:")
    for file_path in key_files:
        if os.path.exists(file_path):
            analysis = analyze_rust_file(file_path)
            print_analysis(analysis)
        else:
            print(f"⚠️  File not found: {file_path}")
    
    # Analyse des crates MCP (refactorés)
    print("\n🆕 MCP CRATES ANALYSIS:")
    mcp_dirs = [
        "crates/mcp-core/src",
        "crates/mcp-tools/src", 
        "crates/mcp-server/src"
    ]
    
    for mcp_dir in mcp_dirs:
        if os.path.exists(mcp_dir):
            print(f"\n📁 {mcp_dir}:")
            rust_files = find_rust_files(mcp_dir)
            for file_path in rust_files:
                analysis = analyze_rust_file(file_path)
                print_analysis(analysis)
    
    # Résumé des structs/functions importantes
    print("\n💡 SUMMARY - Key Components to Migrate:")
    print("- SafeFileWriter (safe_write.rs) → Secure file operations")
    print("- PathSecurity functions (path_security.rs) → Path validation") 
    print("- FileOps functions (file_ops.rs) → File operations")
    print("- Error types (errors.rs) → Error handling")
    print("- Config structures → Configuration management")

if __name__ == "__main__":
    main()

