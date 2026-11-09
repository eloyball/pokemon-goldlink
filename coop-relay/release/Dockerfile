# The GoldLink relay in a container, from the binary beside this file (README.md, "Docker").
#   docker build -t coop-relay .
#   docker run -d --name coop-relay --restart unless-stopped -p 7777:7777/udp -v coop-relay:/data coop-relay
FROM alpine:3.20
COPY coop-relay /usr/local/bin/coop-relay
RUN adduser -D -H -u 10001 relay && mkdir /data && chown relay /data
USER relay
VOLUME /data
EXPOSE 7777/udp
ENTRYPOINT ["/usr/local/bin/coop-relay", "--bind", "0.0.0.0:7777", "--save-dir", "/data"]
