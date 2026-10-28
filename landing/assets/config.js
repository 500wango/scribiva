/*
 * Scribiva landing page — 上线前唯一需要修改的配置文件（所有页面共用）。
 * The single place to edit launch values. Every page reads this file.
 *
 * 待定价确认 / Pricing pending approval:
 *   free_quota、price_monthly、price_yearly、pro_quota 均为占位值（PRD §11.1），
 *   以服务端与 Stripe 中的真实价格为准，上线前必须改成批准后的数值。
 */
window.SCRIBIVA_CONFIG = {
  // 联系方式 / Contact
  contact_email: "support@[域名]",

  // 经营主体（服务条款、隐私政策中使用）/ Operating entity
  legal_entity: "[经营主体名称 / Legal entity name]",
  legal_address: "[经营主体地址 / Registered address]",
  legal_effective_date: "[生效日期 / Effective date]",

  // 定价（待定价确认）/ Pricing (pending approval)
  free_quota_zh: "30 分钟/月",
  free_quota_en: "30 min / month",
  price_monthly_zh: "$8/月",
  price_monthly_en: "$8 / month",
  price_yearly_zh: "$69/年",
  price_yearly_en: "$69 / year",
  pro_quota_zh: "[Pro 月度额度待定]",
  pro_quota_en: "[Pro monthly quota TBD]",

  // 下载 / Downloads
  // release_state: "coming_soon" 显示“即将发布”且按钮不可点；改为 "available" 并填写 url 后变为下载链接。
  release_state: "coming_soon",
  version: "[版本号 / version]",
  windows_url: "",
  windows_sha256: "[SHA-256 待发布 / pending]",
  linux_deb_url: "",
  linux_deb_sha256: "[SHA-256 待发布 / pending]"
};
