// VirtualizedGrid 共享类型与桶导出。
export const BREAKPOINTS = {
    sm: 640,
    md: 768,
    lg: 960,
    xl: 1280,
    '2xl': 1536,
} as const;

export type Breakpoint = keyof typeof BREAKPOINTS;
export type ResponsiveColumns = Partial<Record<Breakpoint | 'default', number>>;

export { default as VirtualizedGrid } from './VirtualizedGrid.vue';
