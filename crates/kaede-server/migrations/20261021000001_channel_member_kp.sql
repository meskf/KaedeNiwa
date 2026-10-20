-- B2：channel_members 增加 key_package 列。
-- 申请者 join 时提交自己的 KP；已有 group 成员从这列取 KP 做 add_member。

ALTER TABLE channel_members ADD COLUMN key_package BLOB;