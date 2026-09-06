<script setup lang="ts" generic="T">
import {
    ref,
    computed,
    watch,
    onMounted,
    onUnmounted,
    useSlots,
} from 'vue';
import { useVirtualizer } from '@tanstack/vue-virtual';
import type { Breakpoint, ResponsiveColumns } from './virtualized-grid';

const BREAKPOINTS = {
    sm: 640,
    md: 768,
    lg: 960,
    xl: 1280,
    '2xl': 1536,
} as const;


interface VirtualizedGridProps<T> {
    items: T[];
    layout?: 'grid' | 'list';
    columns: ResponsiveColumns;
    estimateItemHeight: number;
    gap?: number;
    overscan?: number;
    getItemKey: (item: T, index: number) => string | number;
    renderItem?: (item: T, index: number) => any;
    footer?: any;
    onReachEnd?: () => void;
    reachEndEnabled?: boolean;
    reachEndOffset?: number;
}

const props = withDefaults(defineProps<VirtualizedGridProps<T>>(), {
    layout: 'grid',
    gap: 16,
    overscan: 4,
    footer: null,
    reachEndEnabled: false,
    reachEndOffset: 1,
});

const emit = defineEmits<{
    (e: 'reachEnd'): void;
}>();

const slots = useSlots();

function getColumnsForWidth(
    width: number,
    columns: ResponsiveColumns,
): number {
    if (width >= BREAKPOINTS['2xl'] && columns['2xl'] !== undefined) return columns['2xl']!;
    if (width >= BREAKPOINTS.xl && columns.xl !== undefined) return columns.xl!;
    if (width >= BREAKPOINTS.lg && columns.lg !== undefined) return columns.lg!;
    if (width >= BREAKPOINTS.md && columns.md !== undefined) return columns.md!;
    if (width >= BREAKPOINTS.sm && columns.sm !== undefined) return columns.sm!;
    return columns.default ?? 1;
}

const containerWidth = ref(typeof window === 'undefined' ? 1024 : window.innerWidth);
const containerRef = ref<HTMLDivElement | null>(null);
const reachEndTriggered = ref(false);

let resizeObserver: ResizeObserver | null = null;
let rafId: number | null = null;

onMounted(() => {
    const el = containerRef.value;
    if (!el) return;

    const updateWidth = () => {
        if (rafId !== null) return;
        rafId = window.requestAnimationFrame(() => {
            rafId = null;
            if (!containerRef.value) return;
            const nextWidth = containerRef.value.clientWidth;
            if (Math.abs(containerWidth.value - nextWidth) >= 2) {
                containerWidth.value = nextWidth;
            }
        });
    };

    updateWidth();

    if (typeof ResizeObserver !== 'undefined') {
        resizeObserver = new ResizeObserver(updateWidth);
        resizeObserver.observe(el);
    }
});

onUnmounted(() => {
    if (rafId !== null) window.cancelAnimationFrame(rafId);
    if (resizeObserver) {
        resizeObserver.disconnect();
        resizeObserver = null;
    }
});

const columnCount = computed(() => {
    if (props.layout === 'list') return 1;
    return Math.max(1, getColumnsForWidth(containerWidth.value, props.columns));
});

const itemRowCount = computed(() => {
    if (props.items.length === 0) return 0;
    return Math.ceil(props.items.length / columnCount.value);
});

const hasFooterRow = computed(() => {
    return Boolean(slots.footer || (props.footer !== null && props.footer !== undefined));
});

const rowCount = computed(() => {
    return itemRowCount.value + (hasFooterRow.value ? 1 : 0);
});

function getVirtualRowKey(rowIndex: number): string {
    if (hasFooterRow.value && rowIndex === itemRowCount.value) {
        return '__virtual-footer__';
    }

    const rowStartIndex = rowIndex * columnCount.value;
    const firstItem = props.items[rowStartIndex];
    if (!firstItem) return `row-empty-${rowIndex}`;

    return `row-${String(props.getItemKey(firstItem, rowStartIndex))}`;
}

const rowVirtualizer = useVirtualizer(
    computed(() => ({
        count: rowCount.value,
        getScrollElement: () => containerRef.value,
        getItemKey: getVirtualRowKey,
        estimateSize: () => props.estimateItemHeight + props.gap,
        measureElement: (element: Element) =>
            element instanceof HTMLElement
                ? element.offsetHeight
                : element.getBoundingClientRect().height,
        overscan: props.overscan,
    }))
);

const virtualRows = computed(() => rowVirtualizer.value.getVirtualItems());
const totalSize = computed(() => rowVirtualizer.value.getTotalSize());

watch(
    [() => props.items.length, virtualRows],
    () => {
        if ((!props.onReachEnd && !slots.reachEnd) || !props.reachEndEnabled || itemRowCount.value === 0) {
            return;
        }

        const rows = virtualRows.value;
        const lastVirtualIndex = rows.length > 0 ? rows[rows.length - 1]!.index : -1;
        const triggerIndex = Math.max(0, itemRowCount.value - 1 - props.reachEndOffset);

        if (lastVirtualIndex < triggerIndex) {
            reachEndTriggered.value = false;
            return;
        }

        if (reachEndTriggered.value) return;

        reachEndTriggered.value = true;
        props.onReachEnd?.();
        emit('reachEnd');
    },
    { immediate: true },
);
</script>

<template>
    <div class="relative h-full min-h-0 w-full">
        <div
            ref="containerRef"
            class="relative h-full w-full overflow-y-auto overscroll-contain rounded-t-3xl"
        >
            <div
                v-if="rowCount > 0"
                class="relative w-full"
                :style="{ height: `${totalSize}px` }"
            >
                <div
                    v-for="virtualRow in virtualRows"
                    :key="String(virtualRow.key)"
                    :data-index="virtualRow.index"
                    :ref="(el) => { if (el) rowVirtualizer.measureElement(el as HTMLElement); }"
                    class="absolute left-0 top-0 w-full"
                    :style="{
                        transform: `translateY(${virtualRow.start}px)`,
                    }"
                >
                    <!-- 底部行 -->
                    <template v-if="hasFooterRow && virtualRow.index === itemRowCount">
                        <slot name="footer">
                            <component
                                :is="footer"
                                v-if="typeof footer === 'function' || typeof footer === 'object'"
                            />
                            <template v-else>{{ footer }}</template>
                        </slot>
                    </template>

                    <!-- 数据行 -->
                    <div
                        v-else
                        class="grid"
                        :style="{
                            gridTemplateColumns: `repeat(${columnCount}, minmax(0, 1fr))`,
                            gap: `${gap}px`,
                            paddingBottom: `${virtualRow.index === itemRowCount - 1 && !hasFooterRow ? 0 : gap}px`,
                        }"
                    >
                        <div
                            v-for="(item, columnIndex) in items.slice(
                                virtualRow.index * columnCount,
                                Math.min(virtualRow.index * columnCount + columnCount, items.length),
                            )"
                            :key="String(getItemKey(item, virtualRow.index * columnCount + columnIndex))"
                            class="min-w-0"
                        >
                            <slot
                                :item="item"
                                :index="virtualRow.index * columnCount + columnIndex"
                            >
                                <component
                                    :is="renderItem(item, virtualRow.index * columnCount + columnIndex)"
                                    v-if="renderItem"
                                />
                            </slot>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>
