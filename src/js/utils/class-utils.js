export function costClass(cost) {
    return cost < 1 ? 'cost-low' : cost < 20 ? 'cost-medium' : 'cost-high';
}

export function costTextClass(cost) {
    return cost < 1 ? 'cost-low-text' : cost < 20 ? 'cost-medium-text' : 'cost-high-text';
}

export function sourceClass(source) {
    if (source === 'Clawdbot' || source === 'OpenClaw') return 'openclaw';
    if (source === 'Claude Desktop') return 'desktop';
    if (source === 'Cursor') return 'cursor';
    if (source === 'Windsurf') return 'windsurf';
    if (source === 'Cline' || source === 'Roo Code') return 'cline';
    if (source === 'Aider') return 'aider';
    if (source === 'Continue') return 'continue';
    if (source && source.startsWith('Codex')) return 'codex';
    return 'claude';
}
