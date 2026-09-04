import { SettingAppearance } from './Appearance';
import { SettingSystem } from './System';
import { SettingAPIKey } from './APIKey';
import { SettingModelPrice } from './ModelPrice';
import { SettingAccount } from './Account';
import { SettingModelSync } from './ModelSync';
import { SettingLog } from './Log';
import { SettingBackup } from './Backup';

// Setting 渲染设置页面正文。
export function Setting() {
    return (
        <div className="h-full min-h-0 overflow-y-auto overscroll-contain rounded-t-3xl pb-24 md:pb-4">
            <div className="columns-1 gap-4 md:columns-2 *:mb-4 *:break-inside-avoid">
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
    );
}
