<script setup lang="ts">
import { inject, ref, watch, onMounted, onBeforeUnmount, nextTick, type HTMLAttributes } from 'vue';
import { cn } from '@/lib/utils';
import { useFloating, type Side, type Align } from './positioning';
import type { SelectContext } from './Select.vue';
import SelectScrollUpButton from './SelectScrollUpButton.vue';
import SelectScrollDownButton from './SelectScrollDownButton.vue';

defineOptions({
    inheritAttrs: false,
});

const props = withDefaults(
    defineProps<{
        position?: 'popper' | 'item-aligned';
        align?: Align;
        side?: Side;
        sideOffset?: number;
        alignOffset?: number;
        collisionPadding?: number;
        avoidCollisions?: boolean;
        class?: HTMLAttributes['class'];
    }>(),
    {
        position: 'popper',
        align: 'center',
        side: 'bottom',
        sideOffset: 4,
        alignOffset: 0,
        collisionPadding: 8,
        avoidCollisions: true,
        class: undefined,
    }
);

const select = inject<SelectContext>('selectContext');
const contentRef = ref<HTMLElement | null>(null);

const isOpen = select ? select.isOpen : ref(false);
const triggerRef = select ? select.triggerRef : ref(null);

const { position: floatingPos } = useFloating(triggerRef, contentRef, isOpen, () => ({
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
        if (select) {
            select.setContentRef(el);
        }
    }
);

// Auto scroll highlighted item into view
watch(
    () => select?.highlightedValue.value,
    async (val) => {
        if (!val || !select) return;
        await nextTick();
        const item = select.registeredItems.value.get(val);
        if (item?.el) {
            item.el.scrollIntoView({ block: 'nearest' });
        }
    }
);

const handlePointerDown = (event: PointerEvent) => {
    if (!select || !select.isOpen.value) return;
    const content = contentRef.value;
    const trigger = select.triggerRef.value;
    const target = event.target as Node | null;

    if (!target) return;
    if (content?.contains(target) || trigger?.contains(target)) {
        return;
    }
    select.setOpen(false);
};

const handleKeyDown = (event: KeyboardEvent) => {
    if (!select || !select.isOpen.value) return;

    if (event.key === 'ArrowDown') {
        event.preventDefault();
        select.navigateHighlight('next');
    } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        select.navigateHighlight('prev');
    } else if (event.key === 'Home') {
        event.preventDefault();
        select.navigateHighlight('first');
    } else if (event.key === 'End') {
        event.preventDefault();
        select.navigateHighlight('last');
    } else if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        const val = select.highlightedValue.value;
        if (val) {
            select.selectValue(val);
        }
    } else if (event.key === 'Escape') {
        event.preventDefault();
        select.setOpen(false);
        select.triggerRef.value?.focus();
    } else if (event.key === 'Tab') {
        select.setOpen(false);
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
            v-show="select?.isOpen.value"
            ref="contentRef"
            role="listbox"
            tabindex="-1"
            data-slot="select-content"
            :data-side="floatingPos.actualSide"
            :data-state="select?.isOpen.value ? 'open' : 'closed'"
            :style="{
                position: 'fixed',
                top: `${floatingPos.y}px`,
                left: `${floatingPos.x}px`,
                zIndex: 50,
                minWidth: `${Math.max(128, floatingPos.triggerWidth)}px`,
                '--radix-select-content-transform-origin': floatingPos.transformOrigin,
                '--radix-select-trigger-width': `${floatingPos.triggerWidth}px`,
                '--radix-select-trigger-height': `${floatingPos.triggerHeight}px`,
                '--radix-select-content-available-height': `${floatingPos.maxHeight}px`,
            }"
            :class="
                cn(
                    'bg-popover text-popover-foreground data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 relative z-50 max-h-[var(--radix-select-content-available-height,300px)] min-w-[8rem] origin-(--radix-select-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-md border shadow-md',
                    position === 'popper' &&
                        'data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1',
                    props.class
                )
            "
            v-bind="$attrs"
        >
            <SelectScrollUpButton />
            <div
                data-slot="select-viewport"
                :class="
                    cn(
                        'p-1',
                        position === 'popper' &&
                            'w-full min-w-[var(--radix-select-trigger-width)] scroll-my-1'
                    )
                "
            >
                <slot />
            </div>
            <SelectScrollDownButton />
        </div>
    </Teleport>
</template>
