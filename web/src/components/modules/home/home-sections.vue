<script setup lang="ts">
import Total from './total.vue';
import Activity from './activity.vue';
import StatsChart from './chart.vue';
import Rank from './rank.vue';

// HomeSections 汇总首页各统计区块, 屏内正文与分享图舞台共用。
</script>

<template>
    <div class="@container/home space-y-6">
        <Total />
        <Activity />
        <StatsChart />
        <Rank />
    </div>
</template>
