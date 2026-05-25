export interface TsWidget {
    render(): string;
}

export function tsHelper(): string {
    return "typescript";
}

export function tsEntry(): string {
    return tsHelper();
}

export function tsIsolated(): number {
    return 7;
}
