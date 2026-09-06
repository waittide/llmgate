<script setup lang="ts">
import { inject, ref, watch, type HTMLAttributes } from 'vue';
import { cn } from '@/lib/utils';
import { useFloating, type Side, type Align } from './positioning';
import type { TooltipContext } from './Tooltip.vue';

defineOptions({
    inheritAttrs: false,
});

const props = withDefaults(
    defineProps<{
        side?: Side;
        align?: Align;
        sideOffset?: number;
        alignOffset?: number;
        collisionPadding?: number;
        avoidCollisions?: boolean;
        hideArrow?: boolean;
        class?: HTMLAttributes['class'];
    }>(),
    {
        side: 'top',
        align: 'center',
        sideOffset: 0,
        alignOffset: 0,
        collisionPadding: 8,
        avoidCollisions: true,
        hideArrow: false,
        class: undefined,
    }
);

const tooltip = inject<TooltipContext>('tooltipContext');
const contentRef = ref<HTMLElement | null>(null);

const isOpen = tooltip ? tooltip.isOpen : ref(false);
const triggerRef = tooltip ? tooltip.triggerRef : ref(null);

const { position } = useFloating(triggerRef, contentRef, isOpen, () => ({
    side: props.side,
    align: props.align,
    sideOffset: props.sideOffset,
    alignOffset: props.alignOffset,
    collisionPadding: props.collisionPadding,
    avoidCollisions: props.avoidCollisions,
}));

watch(
    () => contentRef.value,
    (el) => {
        if (tooltip) {
            tooltip.setContentRef(el);
        }
    }
);
</script>

<template>
    <Teleport to="body">
        <div
            v-if="tooltip?.isOpen.value"
            ref="contentRef"
            role="tooltip"
            tabindex="-1"
            data-slot="tooltip-content"
            :data-side="position.actualSide"
            data-state="open"
            :style="{
                position: 'fixed',
                top: `${position.y}px`,
                left: `${position.x}px`,
                zIndex: 50,
                '--radix-tooltip-content-transform-origin': position.transformOrigin,
            }"
            :class="
                cn(
                    'z-50 w-fit origin-(--radix-tooltip-content-transform-origin) animate-in rounded-md bg-foreground px-3 py-1.5 text-xs text-balance text-background fade-in-0 zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 pointer-events-none relative',
                    props.class
                )
            "
            v-bind="$attrs"
        >
            <slot />
            <!-- Arrow -->
            <svg
                v-if="!hideArrow"
                data-slot="tooltip-arrow"
                width="10"
                height="5"
                viewBox="0 0 30 10"
                preserveAspectRatio="none"
                :class="
                    cn('fill-foreground absolute pointer-events-none', {
                        'bottom-[-5px] left-1/2 -translate-x-1/2 rotate-180': position.actualSide === 'top',
                        'top-[-5px] left-1/2 -translate-x-1/2': position.actualSide === 'bottom',
                        'right-[-8px] top-1/2 -translate-y-1/2 rotate-90': position.actualSide === 'left',
                        'left-[-8px] top-1/2 -translate-y-1/2 -rotate-90': position.actualSide === 'right',
                    })
                "
            >
                <polygon points="0,0 30,0 15,10" />
            </svg>
        </div>
    </Teleport>
</template>
