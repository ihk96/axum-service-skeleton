-- 예제 테이블. domain/example.rs, store/example.rs와 짝이며 지울 대상이다.
create table examples (
    id         uuid        primary key default gen_random_uuid(),
    name       text        not null unique,
    created_at timestamptz not null default now()
);
