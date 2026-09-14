<script setup lang="ts">
import {
    cloneVNode,
    Comment,
    computed,
    defineComponent,
    Fragment,
    inject,
    nextTick,
    onMounted,
    ref,
    Static,
    Text,
    useAttrs,
    useSlots,
    type ComponentPublicInstance,
    type VNode,
    type VNodeTypes,
} from 'vue';
import type { TooltipContext } from './Tooltip.vue';

defineOptions({
    inheritAttrs: false,
});

const props = withDefaults(
    defineProps<{
        asChild?: boolean;
    }>(),
    {
        asChild: false,
    }
);

const attrs = useAttrs();
const slots = useSlots();
const tooltip = inject<TooltipContext>('tooltipContext');
const triggerEl = ref<HTMLElement | null>(null);

const setTriggerEl = (el: Element | ComponentPublicInstance | null) => {
    triggerEl.value = el && '$el' in el ? ((el as ComponentPublicInstance).$el as HTMLElement) : ((el as HTMLElement | null) ?? null);
};

// as-child 时直接把插槽内第一个元素/组件节点作为触发器（Radix 语义），
// 不渲染额外包裹层：包裹层会收缩包裹子内容，破坏 h3.truncate 等
// 依赖父宽度约束的截断/布局样式（如分组卡片长名称溢出）。
const INVALID_TYPES: VNodeTypes[] = [Fragment, Comment, Text, Static];

const handleMouseEnter = () => {
    tooltip?.show();
};

const handleMouseLeave = () => {
    tooltip?.hide();
};

const handleFocus = () => {
    tooltip?.show();
};

const handleBlur = () => {
    tooltip?.hide();
};

const triggerProps = computed(() => ({
    'data-slot': 'tooltip-trigger',
    'data-state': tooltip?.isOpen.value ? 'open' : 'closed',
    onMouseenter: handleMouseEnter,
    onMouseleave: handleMouseLeave,
    onFocus: handleFocus,
    onBlur: handleBlur,
}));

const TriggerChild = defineComponent({
    setup() {
        return () => {
            if (!props.asChild) return null;
            const children = slots.default?.() ?? [];
            const vnode = children.find((v): v is VNode => !!v && v.type !== Comment && !INVALID_TYPES.includes(v.type));
            if (!vnode) return null;
            return cloneVNode(vnode, { ...attrs, ...triggerProps.value, ref: setTriggerEl });
        };
    },
});

onMounted(async () => {
    await nextTick();
    if (tooltip && triggerEl.value) {
        tooltip.setTriggerRef(triggerEl.value);
    }
});
</script>

<template>
    <TriggerChild v-if="asChild" />
    <div
        v-else
        ref="triggerEl"
        data-slot="tooltip-trigger"
        :data-state="tooltip?.isOpen.value ? 'open' : 'closed'"
        class="inline-block"
        v-bind="attrs"
        @mouseenter="handleMouseEnter"
        @mouseleave="handleMouseLeave"
        @focus="handleFocus"
        @blur="handleBlur"
    >
        <slot />
    </div>
</template>
