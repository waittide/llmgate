<script setup lang="ts">
import { ref, provide, watch, computed, toRef, type Ref, type ComputedRef } from 'vue';

export interface SelectItemData {
    value: string;
    label: string;
    disabled: boolean;
    el: HTMLElement | null;
}

export interface SelectContext {
    selectedValue: Ref<string | undefined>;
    isOpen: Ref<boolean>;
    disabled: Ref<boolean>;
    highlightedValue: Ref<string | null>;
    registeredItems: Ref<Map<string, SelectItemData>>;
    selectedLabel: ComputedRef<string | undefined>;
    triggerRef: Ref<HTMLElement | null>;
    contentRef: Ref<HTMLElement | null>;
    setTriggerRef: (el: HTMLElement | null) => void;
    setContentRef: (el: HTMLElement | null) => void;
    setOpen: (open: boolean) => void;
    selectValue: (value: string) => void;
    registerItem: (data: SelectItemData) => void;
    unregisterItem: (value: string) => void;
    updateItem: (value: string, data: Partial<SelectItemData>) => void;
    setHighlightedValue: (value: string | null) => void;
    navigateHighlight: (direction: 'next' | 'prev' | 'first' | 'last') => void;
}

const props = withDefaults(
    defineProps<{
        modelValue?: string;
        value?: string;
        defaultValue?: string;
        open?: boolean;
        defaultOpen?: boolean;
        disabled?: boolean;
        name?: string;
        required?: boolean;
    }>(),
    {
        modelValue: undefined,
        value: undefined,
        defaultValue: undefined,
        open: undefined,
        defaultOpen: false,
        disabled: false,
        name: undefined,
        required: false,
    }
);

const emit = defineEmits<{
    (e: 'update:modelValue', value: string): void;
    (e: 'update:value', value: string): void;
    (e: 'valueChange', value: string): void;
    (e: 'update:open', open: boolean): void;
    (e: 'openChange', open: boolean): void;
}>();

const initialValue = props.modelValue !== undefined ? props.modelValue : props.value !== undefined ? props.value : props.defaultValue;
const internalValue = ref<string | undefined>(initialValue);

watch(
    () => props.modelValue,
    (val) => {
        if (val !== undefined) internalValue.value = val;
    }
);

watch(
    () => props.value,
    (val) => {
        if (val !== undefined) internalValue.value = val;
    }
);

const selectedValue = computed(() => {
    if (props.modelValue !== undefined) return props.modelValue;
    if (props.value !== undefined) return props.value;
    return internalValue.value;
});

const internalOpen = ref(props.open !== undefined ? props.open : props.defaultOpen);

watch(
    () => props.open,
    (val) => {
        if (val !== undefined) internalOpen.value = val;
    }
);

const isOpen = ref(internalOpen.value);

watch(
    () => [props.open, internalOpen.value],
    () => {
        isOpen.value = props.open !== undefined ? props.open : internalOpen.value;
    }
);

const highlightedValue = ref<string | null>(null);
const registeredItems = ref<Map<string, SelectItemData>>(new Map());

const selectedLabel = computed(() => {
    const val = selectedValue.value;
    if (val === undefined) return undefined;
    return registeredItems.value.get(val)?.label;
});

const triggerRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);

const setTriggerRef = (el: HTMLElement | null) => {
    triggerRef.value = el;
};

const setContentRef = (el: HTMLElement | null) => {
    contentRef.value = el;
};

const setOpen = (open: boolean) => {
    if (props.disabled) return;
    if (props.open === undefined) {
        internalOpen.value = open;
    }
    emit('update:open', open);
    emit('openChange', open);

    if (open) {
        // Default highlight to selected or first item
        if (selectedValue.value && registeredItems.value.has(selectedValue.value)) {
            highlightedValue.value = selectedValue.value;
        } else {
            navigateHighlight('first');
        }
    } else {
        highlightedValue.value = null;
    }
};

const selectValue = (val: string) => {
    if (props.disabled) return;
    internalValue.value = val;
    emit('update:modelValue', val);
    emit('update:value', val);
    emit('valueChange', val);
    setOpen(false);
    triggerRef.value?.focus();
};

const registerItem = (data: SelectItemData) => {
    registeredItems.value.set(data.value, data);
};

const unregisterItem = (val: string) => {
    registeredItems.value.delete(val);
};

const updateItem = (val: string, data: Partial<SelectItemData>) => {
    const existing = registeredItems.value.get(val);
    if (existing) {
        registeredItems.value.set(val, { ...existing, ...data });
    }
};

const setHighlightedValue = (val: string | null) => {
    highlightedValue.value = val;
};

const getValidItems = (): SelectItemData[] => {
    const items: SelectItemData[] = [];
    for (const item of registeredItems.value.values()) {
        if (!item.disabled) {
            items.push(item);
        }
    }
    return items;
};

const navigateHighlight = (direction: 'next' | 'prev' | 'first' | 'last') => {
    const items = getValidItems();
    if (items.length === 0) return;

    if (direction === 'first') {
        highlightedValue.value = items[0].value;
        return;
    }
    if (direction === 'last') {
        highlightedValue.value = items[items.length - 1].value;
        return;
    }

    const currentIndex = items.findIndex((item) => item.value === highlightedValue.value);

    if (direction === 'next') {
        if (currentIndex < 0 || currentIndex >= items.length - 1) {
            highlightedValue.value = items[0].value;
        } else {
            highlightedValue.value = items[currentIndex + 1].value;
        }
    } else if (direction === 'prev') {
        if (currentIndex <= 0) {
            highlightedValue.value = items[items.length - 1].value;
        } else {
            highlightedValue.value = items[currentIndex - 1].value;
        }
    }
};

provide('selectContext', {
    selectedValue,
    isOpen,
    disabled: toRef(props, 'disabled'),
    highlightedValue,
    registeredItems,
    selectedLabel,
    triggerRef,
    contentRef,
    setTriggerRef,
    setContentRef,
    setOpen,
    selectValue,
    registerItem,
    unregisterItem,
    updateItem,
    setHighlightedValue,
    navigateHighlight,
});
</script>

<template>
    <div data-slot="select" class="contents">
        <slot :open="isOpen" :value="selectedValue" />
    </div>
</template>
