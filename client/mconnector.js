var grpc = require('@grpc/grpc-js');
// const fs = require('fs');

var path = require("path");
var PROTO_PATH = path.join(__dirname, '..', 'proto', 'mtransaction.proto');

var protoLoader = require('@grpc/proto-loader');
var packageDefinition = protoLoader.loadSync(
    PROTO_PATH,
    {keepCase: true,
     longs: String,
     enums: String,
     defaults: true,
     oneofs: true
    });
var validator_proto = grpc.loadPackageDefinition(packageDefinition).validator;

function main() {
    // const ssl_creds = grpc.credentials.createSsl(
    //     fs.readFileSync(path.join(__dirname, '..', 'cert', 'ca-cert.cer')),
    //     fs.readFileSync(path.join(__dirname, '..', 'cert', 'client-key.cer')),
    //     fs.readFileSync(path.join(__dirname, '..', 'cert', 'client-cert.cer')),
    // );
    // var client = new validator_proto.MTransaction('192.168.200.176:50051', ssl_creds);
    var client = new validator_proto.MTransaction('192.168.200.176:50051', grpc.credentials.createInsecure());

    client.EchoStream({message: 'world'}, function(err, response) {
        if (err) throw err;
        console.log('Greeting:', response.message);
        // client.close();
    });
}

main();