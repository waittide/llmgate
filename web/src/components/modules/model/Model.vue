<script setup lang="ts">
import { computed } from 'vue';
import { useModelList, type ModelInfo } from '@/api/model';
import { pageActionsStore } from '@/components/common/page-actions-store';
import VirtualizedGrid from '@/components/common/VirtualizedGrid.vue';
import ModelItem from './Item.vue';

// Model 渲染模型列表正文。
const { data: models } = useModelList();

const searchTerm = computed(() => pageActionsStore.searchTerms.model || '');
const layout = computed(() => pageActionsStore.layouts.model || 'grid');
const sortOrder = computed(() => (pageActionsStore.sortOrders.model === 'desc' ? 'desc' : 'asc'));
const filter = computed(() => pageActionsStore.modelFilter);

const sortedModels = computed(() => {
    if (!models.value) return [];
    return [...models.value].sort((a, b) =>
        sortOrder.value === 'asc' ? a.name.localeCompare(b.name) : b.name.localeCompare(a.name)
    );
});

const visibleModels = computed(() => {
    const term = searchTerm.value.toLowerCase().trim();
    const byName = !term
        ? sortedModels.value
        : sortedModels.value.filter((m) => m.name.toLowerCase().includes(term));

    const hasPricing = (model: ModelInfo) =>
        model.input + model.output + model.cache_read + model.cache_write > 0;

    if (filter.value === 'priced') {
        return byName.filter(hasPricing);
    }
    if (filter.value === 'free') {
        return byName.filter((m) => !hasPricing(m));
    }

    return byName;
});
</script>

<template>
    <VirtualizedGrid
        :items="visibleModels"
        :layout="layout"
        :columns="{ default: 1, md: 2, lg: 3 }"
        :estimate-item-height="112"
        :get-item-key="(model: ModelInfo) => `model-${model.name}`"
    >
        <template #default="{ item }">
            <ModelItem :model="item" :layout="layout" />
        </template>
    </VirtualizedGrid>
</template>
