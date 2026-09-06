<script setup lang="ts">
import SettingAppearance from './Appearance.vue';
import SettingAccount from './Account.vue';
import SettingSystem from './System.vue';
import SettingLog from './Log.vue';
import SettingModelPrice from './ModelPrice.vue';
import SettingAPIKey from './APIKey.vue';
import SettingModelSync from './ModelSync.vue';
import SettingBackup from './Backup.vue';
</script>

<template>
    <!-- Setting 渲染设置页面正文 -->
    <div class="h-full min-h-0 overflow-y-auto overscroll-contain rounded-t-3xl pb-24 md:pb-4">
        <div class="columns-1 gap-4 md:columns-2 *:mb-4 *:break-inside-avoid">
            <SettingAppearance />
            <SettingAccount />
            <SettingSystem />
            <SettingLog />
            <SettingModelPrice />
            <SettingAPIKey />
            <SettingModelSync />
            <SettingBackup />
        </div>
    </div>
</template>
