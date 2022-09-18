var grpc = require('@grpc/grpc-js');

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
  var client = new validator_proto.MTransaction('https://192.168.200.176:50051/',
                                       grpc.credentials.createInsecure());

  client.Echo({name: user}, function(err, response) {
    if (err) throw err;
    console.log('Greeting:', response.message);
    client.close();
  });
}

main();