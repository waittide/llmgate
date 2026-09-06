<script setup lang="ts">
import { computed } from 'vue';
import { useChannelList } from '@/api/channel';
import { useTranslations } from '@/composables/i18n';
import { formatCount, formatMoney } from '@/lib/utils';
import { homeViewStore } from './store';
import RankCard, { type RankItem } from './RankCard.vue';

// Rank 并列渠道榜和模型榜, 两榜各自独立排序。
const { data: channels } = useChannelList();
const { t } = useTranslations('home.rank');

const channelItems = computed<RankItem[]>(() =>
    (channels.value ?? []).map((channel) => ({
        id: `channel-${channel.raw.id}`,
        name: channel.raw.name,
        formatted: channel.formatted,
    }))
);

// 每个模型要现算 5 项格式化, 模型总数可观, 只应随渠道数据变化重算。
const modelItems = computed<RankItem[]>(() =>
    (channels.value ?? []).flatMap((channel) =>
        channel.raw.models.map((model) => ({
            id: `model-${model.id}`,
            name: model.name,
            channelName: channel.raw.name,
            formatted: {
                total_cost: formatMoney(model.input_cost + model.output_cost),
                total_token: formatCount(model.input_token + model.output_token),
                request_count: formatCount(model.request_success + model.request_failed),
                request_success: formatCount(model.request_success),
                request_failed: formatCount(model.request_failed),
            },
        }))
    )
);
</script>

<template>
    <div class="grid grid-cols-1 @3xl/home:grid-cols-2 gap-4">
        <RankCard
            :title="t('channel')"
            :items="channelItems"
            :sort-mode="homeViewStore.channelRankSortMode"
            @update:sort-mode="homeViewStore.setChannelRankSortMode"
            :hide-channel-name="homeViewStore.isChannelNameHidden"
        />
        <RankCard
            :title="t('model')"
            :items="modelItems"
            :sort-mode="homeViewStore.modelRankSortMode"
            @update:sort-mode="homeViewStore.setModelRankSortMode"
            :hide-channel-name="homeViewStore.isChannelNameHidden"
        />
    </div>
</template>
