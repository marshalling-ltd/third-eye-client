# \IntermagnetDataHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**get_intermagnet_data_handler**](IntermagnetDataHandlerApi.md#get_intermagnet_data_handler) | **GET** /api/v1/intermagnet-data/{catalog_id} | 



## get_intermagnet_data_handler

> models::PagedResponseTimeSeriesIntermagnetDataModel get_intermagnet_data_handler(catalog_id, start, limit, agg_level, normalize, differentiate)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**catalog_id** | **uuid::Uuid** | Intermagnet catalog id | [required] |
**start** | Option<**String**> | The start time for the search. If not specified, it defaults to the earliest available time in the data. |  |
**limit** | Option<**i32**> | Maximum number of records to return. Defaults to 10000. |  |
**agg_level** | Option<[**TimeBucket**](.md)> | Level of data aggregation. Default is per minute. |  |
**normalize** | Option<**bool**> | If set to true, the variables will be centered to have a zero mean. Defaults to false. |  |
**differentiate** | Option<**bool**> | If set to true, the time series will be differenced, transforming each value to the change from its previous value (i.e., x_t - x_(t-1)). Defaults to false. |  |

### Return type

[**models::PagedResponseTimeSeriesIntermagnetDataModel**](PagedResponseTimeSeries_IntermagnetDataModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

