<script setup lang="ts">
import { computed } from 'vue';
import JsonView from '@/components/ui/json-view.vue';

interface Props {
    content: string | object | undefined;
    fallbackText: string;
    collapsed?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
    collapsed: true,
});

const parsed = computed(() => {
    if (props.content === undefined || props.content === '') return null;
    if (typeof props.content !== 'string') return { isJson: true, data: props.content };
    try {
        return { isJson: true, data: JSON.parse(props.content) as object };
    } catch {
        return { isJson: false, data: props.content };
    }
});
</script>

<template>
    <pre
        v-if="!parsed"
        class="p-4 text-xs text-muted-foreground whitespace-pre-wrap break-words leading-relaxed"
    >{{ fallbackText }}</pre>

    <pre
        v-else-if="!parsed.isJson"
        class="p-4 text-xs text-muted-foreground whitespace-pre-wrap break-words font-mono leading-relaxed select-text animate-in fade-in duration-200"
    >{{ parsed.data as string }}</pre>

    <div v-else class="p-4 animate-in fade-in duration-200">
        <JsonView
            :value="parsed.data"
            :display-data-types="false"
            :display-object-size="false"
            :collapsed="collapsed"
        />
    </div>
</template>
