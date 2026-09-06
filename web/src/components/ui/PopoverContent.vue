<script setup lang="ts">
import { inject, ref, onMounted, onBeforeUnmount, watch, type HTMLAttributes } from 'vue';
import { cn } from '@/lib/utils';
import { useFloating, type Side, type Align } from './positioning';
import type { PopoverContext } from './Popover.vue';

defineOptions({
    inheritAttrs: false,
});

const props = withDefaults(
    defineProps<{
        align?: Align;
        side?: Side;
        sideOffset?: number;
        alignOffset?: number;
        collisionPadding?: number;
        avoidCollisions?: boolean;
        class?: HTMLAttributes['class'];
    }>(),
    {
        align: 'center',
        side: 'bottom',
        sideOffset: 4,
        alignOffset: 0,
        collisionPadding: 8,
        avoidCollisions: true,
        class: undefined,
    }
);

const popover = inject<PopoverContext>('popoverContext');
const contentRef = ref<HTMLElement | null>(null);

const isOpen = popover ? popover.isOpen : ref(false);
const triggerRef = popover ? popover.triggerRef : ref(null);

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
        if (popover) {
            popover.setContentRef(el);
        }
    }
);

const handlePointerDown = (event: PointerEvent) => {
    if (!popover || !popover.isOpen.value) return;
    const content = contentRef.value;
    const trigger = popover.triggerRef.value;
    const target = event.target as Node | null;

    if (!target) return;
    if (content?.contains(target) || trigger?.contains(target)) {
        return;
    }
    popover.setOpen(false);
};

const handleKeyDown = (event: KeyboardEvent) => {
    if (!popover || !popover.isOpen.value) return;
    if (event.key === 'Escape') {
        popover.setOpen(false);
    }
};

onMounted(() => {
    document.addEventListener('pointerdown', handlePointerDown, true);
    document.addEventListener('keydown', handleKeyDown);
});

onBeforeUnmount(() => {
    document.removeEventListener('pointerdown', handlePointerDown, true);
    document.removeEventListener('keydown', handleKeyDown);
});
</script>

<template>
    <Teleport to="body">
        <div
            v-if="popover?.isOpen.value"
            ref="contentRef"
            role="dialog"
            tabindex="-1"
            data-slot="popover-content"
            :data-side="position.actualSide"
            data-state="open"
            :style="{
                position: 'fixed',
                top: `${position.y}px`,
                left: `${position.x}px`,
                zIndex: 50,
                '--radix-popover-content-transform-origin': position.transformOrigin,
            }"
            :class="
                cn(
                    'bg-popover text-popover-foreground data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 z-50 w-72 origin-(--radix-popover-content-transform-origin) rounded-md border p-4 shadow-md outline-hidden',
                    props.class
                )
            "
            v-bind="$attrs"
        >
            <slot />
        </div>
    </Teleport>
</template>
