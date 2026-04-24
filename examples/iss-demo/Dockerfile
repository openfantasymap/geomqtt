FROM alpine:3.20
RUN apk add --no-cache curl jq redis
COPY iss-demo.sh /usr/local/bin/iss-demo
RUN chmod +x /usr/local/bin/iss-demo
ENTRYPOINT ["/usr/local/bin/iss-demo"]
