import { ref, watch, onMounted, onBeforeUnmount, nextTick, type Ref } from 'vue';

export type Side = 'top' | 'bottom' | 'left' | 'right';
export type Align = 'start' | 'center' | 'end';

export interface FloatingOptions {
    side?: Side;
    align?: Align;
    sideOffset?: number;
    alignOffset?: number;
    collisionPadding?: number;
    avoidCollisions?: boolean;
}

export interface PositionResult {
    x: number;
    y: number;
    actualSide: Side;
    actualAlign: Align;
    transformOrigin: string;
    triggerWidth: number;
    triggerHeight: number;
    maxHeight: number;
}

export function computeFloatingPosition(
    triggerRect: DOMRect,
    contentRect: { width: number; height: number },
    options: FloatingOptions = {}
): PositionResult {
    const {
        side = 'bottom',
        align = 'center',
        sideOffset = 4,
        alignOffset = 0,
        collisionPadding = 8,
        avoidCollisions = true,
    } = options;

    const viewportWidth =
        typeof window !== 'undefined'
            ? window.innerWidth || document.documentElement.clientWidth
            : 1024;
    const viewportHeight =
        typeof window !== 'undefined'
            ? window.innerHeight || document.documentElement.clientHeight
            : 768;

    let actualSide: Side = side;

    if (avoidCollisions) {
        const spaceBelow = viewportHeight - triggerRect.bottom - sideOffset - collisionPadding;
        const spaceAbove = triggerRect.top - sideOffset - collisionPadding;
        const spaceRight = viewportWidth - triggerRect.right - sideOffset - collisionPadding;
        const spaceLeft = triggerRect.left - sideOffset - collisionPadding;

        if (side === 'bottom') {
            if (contentRect.height > spaceBelow && spaceAbove > spaceBelow) {
                actualSide = 'top';
            }
        } else if (side === 'top') {
            if (contentRect.height > spaceAbove && spaceBelow > spaceAbove) {
                actualSide = 'bottom';
            }
        } else if (side === 'right') {
            if (contentRect.width > spaceRight && spaceLeft > spaceRight) {
                actualSide = 'left';
            }
        } else if (side === 'left') {
            if (contentRect.width > spaceLeft && spaceRight > spaceLeft) {
                actualSide = 'right';
            }
        }
    }

    let x = 0;
    let y = 0;

    if (actualSide === 'bottom' || actualSide === 'top') {
        if (actualSide === 'bottom') {
            y = triggerRect.bottom + sideOffset;
        } else {
            y = triggerRect.top - contentRect.height - sideOffset;
        }

        if (align === 'start') {
            x = triggerRect.left + alignOffset;
        } else if (align === 'end') {
            x = triggerRect.right - contentRect.width - alignOffset;
        } else {
            x = triggerRect.left + (triggerRect.width - contentRect.width) / 2 + alignOffset;
        }

        x = Math.max(collisionPadding, Math.min(x, viewportWidth - contentRect.width - collisionPadding));
    } else {
        if (actualSide === 'right') {
            x = triggerRect.right + sideOffset;
        } else {
            x = triggerRect.left - contentRect.width - sideOffset;
        }

        if (align === 'start') {
            y = triggerRect.top + alignOffset;
        } else if (align === 'end') {
            y = triggerRect.bottom - contentRect.height - alignOffset;
        } else {
            y = triggerRect.top + (triggerRect.height - contentRect.height) / 2 + alignOffset;
        }

        y = Math.max(collisionPadding, Math.min(y, viewportHeight - contentRect.height - collisionPadding));
    }

    let transformOrigin = 'center';
    if (actualSide === 'bottom') {
        transformOrigin = align === 'start' ? 'top left' : align === 'end' ? 'top right' : 'top center';
    } else if (actualSide === 'top') {
        transformOrigin = align === 'start' ? 'bottom left' : align === 'end' ? 'bottom right' : 'bottom center';
    } else if (actualSide === 'left') {
        transformOrigin = align === 'start' ? 'top right' : align === 'end' ? 'bottom right' : 'center right';
    } else if (actualSide === 'right') {
        transformOrigin = align === 'start' ? 'top left' : align === 'end' ? 'bottom left' : 'center left';
    }

    const maxHeight = actualSide === 'bottom'
        ? Math.max(100, viewportHeight - triggerRect.bottom - sideOffset - collisionPadding)
        : actualSide === 'top'
        ? Math.max(100, triggerRect.top - sideOffset - collisionPadding)
        : Math.max(100, viewportHeight - collisionPadding * 2);

    return {
        x: Math.round(x),
        y: Math.round(y),
        actualSide,
        actualAlign: align,
        transformOrigin,
        triggerWidth: Math.round(triggerRect.width),
        triggerHeight: Math.round(triggerRect.height),
        maxHeight: Math.round(maxHeight),
    };
}

export function useFloating(
    triggerRef: Ref<HTMLElement | null>,
    contentRef: Ref<HTMLElement | null>,
    isOpen: Ref<boolean>,
    getOptions: () => FloatingOptions
) {
    const position = ref<PositionResult>({
        x: 0,
        y: 0,
        actualSide: 'bottom',
        actualAlign: 'center',
        transformOrigin: 'top center',
        triggerWidth: 0,
        triggerHeight: 0,
        maxHeight: 300,
    });

    const update = () => {
        if (!isOpen.value || !triggerRef.value || !contentRef.value) return;
        const triggerRect = triggerRef.value.getBoundingClientRect();
        const contentRect = contentRef.value.getBoundingClientRect();
        position.value = computeFloatingPosition(triggerRect, contentRect, getOptions());
    };

    let resizeObserver: ResizeObserver | null = null;

    const setupListeners = () => {
        if (typeof window === 'undefined') return;
        window.addEventListener('resize', update, { passive: true });
        window.addEventListener('scroll', update, { passive: true, capture: true });
        if (typeof ResizeObserver !== 'undefined') {
            resizeObserver = new ResizeObserver(() => update());
            if (triggerRef.value) resizeObserver.observe(triggerRef.value);
            if (contentRef.value) resizeObserver.observe(contentRef.value);
        }
    };

    const cleanupListeners = () => {
        if (typeof window === 'undefined') return;
        window.removeEventListener('resize', update);
        window.removeEventListener('scroll', update, true);
        if (resizeObserver) {
            resizeObserver.disconnect();
            resizeObserver = null;
        }
    };

    watch(
        () => isOpen.value,
        async (open) => {
            if (open) {
                await nextTick();
                update();
                setupListeners();
            } else {
                cleanupListeners();
            }
        },
        { immediate: false }
    );

    onMounted(() => {
        if (isOpen.value) {
            void nextTick(() => {
                update();
                setupListeners();
            });
        }
    });

    onBeforeUnmount(() => {
        cleanupListeners();
    });

    return {
        position,
        update,
    };
}
