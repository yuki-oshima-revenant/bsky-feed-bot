FROM amd64/rust:1.73 as builder
WORKDIR /usr/src/bsky-feed-bot
COPY . .
RUN cargo build --release

FROM public.ecr.aws/lambda/provided:al2023.2023.11.18.01 as bsky-feed-bot-lambda
COPY --from=builder \
    /usr/src/bsky-feed-bot/target/release/bsky-feed-bot \
    ${LAMBDA_RUNTIME_DIR}/bootstrap
CMD [ "lambda-handler" ]

FROM public.ecr.aws/lambda/provided:al2023.2023.11.18.01 as test
COPY --from=builder \
    /usr/src/bsky-feed-bot/target/release/test \
    ${LAMBDA_RUNTIME_DIR}/bootstrap
CMD [ "lambda-handler" ]
