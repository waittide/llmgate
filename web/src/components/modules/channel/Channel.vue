<script setup lang="ts">
import { computed } from 'vue';
import { useChannelList, type ChannelListItem } from '@/api/channel';
import { pageActionsStore } from '@/components/common/page-actions-store';
import VirtualizedGrid from '@/components/common/VirtualizedGrid.vue';
import Card from './Card.vue';

// Channel 渲染渠道列表正文。
const { data: channelsData } = useChannelList();

const searchTerm = computed(() => pageActionsStore.searchTerms.channel || '');
const layout = computed(() => pageActionsStore.layouts.channel || 'grid');
const sortOrder = computed(() => (pageActionsStore.sortOrders.channel === 'desc' ? 'desc' : 'asc'));
const filter = computed(() => pageActionsStore.channelFilter);

// 先按搜索词和启用状态过滤, 再按名称排序
const visibleChannels = computed<ChannelListItem[]>(() => {
    const term = searchTerm.value.toLowerCase().trim();
    const matched = (channelsData.value ?? []).filter((item) => {
        if (term && !item.raw.name.toLowerCase().includes(term)) return false;
        if (filter.value === 'enabled') return item.raw.enabled;
        if (filter.value === 'disabled') return !item.raw.enabled;
        return true;
    });

    return matched.sort((a, b) =>
        sortOrder.value === 'asc'
            ? a.raw.name.localeCompare(b.raw.name)
            : b.raw.name.localeCompare(a.raw.name)
    );
});
</script>

<template>
    <VirtualizedGrid
        :items="visibleChannels"
        :layout="layout"
        :columns="{ default: 1, md: 2, lg: 3 }"
        :estimate-item-height="216"
        :get-item-key="(item: ChannelListItem) => `channel-${item.raw.id}`"
    >
        <template #default="{ item }">
            <Card :channel="item.raw" :stats="item.formatted" :layout="layout" />
        </template>
    </VirtualizedGrid>
</template>
